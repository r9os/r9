use crate::config::Configuration;
use config::{apply_to_build_step, apply_to_clippy_step, apply_to_qemu_step};
use std::{
    env, fmt,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{self, Command, Stdio},
    str::FromStr,
    sync::mpsc::{self, RecvTimeoutError},
    thread,
    time::{Duration, Instant},
};
use target_lexicon::Triple;

mod config;

type DynError = Box<dyn std::error::Error>;
type Result<T> = std::result::Result<T, DynError>;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Profile {
    Debug,
    Release,
}

impl Profile {
    fn from(matches: &clap::ArgMatches) -> Self {
        if matches.get_flag("release") { Profile::Release } else { Profile::Debug }
    }

    fn dir(&self) -> &'static str {
        match self {
            Profile::Debug => "debug",
            Profile::Release => "release",
        }
    }
}

impl fmt::Display for Profile {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{self:?}")
    }
}

#[derive(Clone, Copy, Debug, PartialEq, clap::ValueEnum)]
enum Arch {
    Aarch64,
    Riscv64,
    X86_64,
}

impl Arch {
    fn from(matches: &clap::ArgMatches) -> Self {
        *matches.get_one::<Arch>("arch").unwrap_or(&Arch::X86_64)
    }

    fn qemu_system(&self) -> String {
        env_or(
            "QEMU",
            match self {
                Arch::Aarch64 => "qemu-system-aarch64",
                Arch::Riscv64 => "qemu-system-riscv64",
                Arch::X86_64 => "qemu-system-x86_64",
            },
        )
    }

    fn target(&self) -> String {
        env_or("TARGET", format!("{}-unknown-none-elf", self.to_string().to_lowercase()).as_str())
    }
}

impl fmt::Display for Arch {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{self:?}")
    }
}

struct RustupState {
    installed_targets: Vec<Triple>,
    curr_toolchain: String,
}

impl RustupState {
    /// Runs rustup command to get a list of all installed toolchains.
    /// Also caches the current toolchain.
    fn new() -> Self {
        Self {
            installed_targets: Self::installed_rustup_targets().unwrap(),
            curr_toolchain: env::var("RUSTUP_TOOLCHAIN").unwrap(),
        }
    }

    /// Call `rustup target list --installed` to get all installed target triples
    fn installed_rustup_targets() -> Result<Vec<Triple>> {
        let output =
            Command::new("rustup").arg("target").arg("list").arg("--installed").output()?;
        if !output.status.success() {
            return Err(String::from_utf8(output.stdout.clone())?.into());
        }

        Ok(String::from_utf8(output.stdout.clone())?.lines().flat_map(Triple::from_str).collect())
    }

    /// For the given arch, return a compatible toolchain triple that is
    /// installed and can be used by cargo check.  It will prefer the default
    /// toolchain if it's a match, otherwise it will look for the
    /// <arch-unknown-linux-gnu> toolchain.
    fn std_supported_target(&self, arch: &str) -> Option<&Triple> {
        let arch = Self::target_arch(arch);
        self.installed_targets.iter().filter(|&t| t.architecture.to_string() == arch).find(|&t| {
            self.curr_toolchain.ends_with(&t.to_string())
                || t.to_string() == arch.to_owned() + "-unknown-linux-gnu"
        })
    }

    /// Return the arch in a form compatible with the supported targets and toolchains
    fn target_arch(arch: &str) -> &str {
        match arch {
            "riscv64" => "riscv64gc",
            _ => arch,
        }
    }
}

fn main() {
    let matches = clap::Command::new("xtask")
        .version("0.1.0")
        .author("The r9 Authors")
        .about("Build support for the r9 operating system")
        .arg_required_else_help(true)
        .subcommand(
            clap::Command::new("build").about("Builds r9").args(&[
                clap::arg!(--release "Build release version").conflicts_with("debug"),
                clap::arg!(--debug "Build debug version (default)").conflicts_with("release"),
                clap::arg!(--arch <arch> "Target architecture")
                    .value_parser(clap::builder::EnumValueParser::<Arch>::new()),
                clap::arg!(--config <name> "Configuration")
                    .value_parser(clap::builder::NonEmptyStringValueParser::new())
                    .default_value("default"),
                clap::arg!(--verbose "Print commands"),
            ]),
        )
        .subcommand(
            clap::Command::new("expand").about("Expands r9 macros").args(&[
                clap::arg!(--release "Build release version").conflicts_with("debug"),
                clap::arg!(--debug "Build debug version (default)").conflicts_with("release"),
                clap::arg!(--arch <arch> "Target architecture")
                    .value_parser(clap::builder::EnumValueParser::<Arch>::new()),
                clap::arg!(--verbose "Print commands"),
            ]),
        )
        .subcommand(
            clap::Command::new("kasm").about("Emits r9 assembler").args(&[
                clap::arg!(--release "Build release version").conflicts_with("debug"),
                clap::arg!(--debug "Build debug version (default)").conflicts_with("release"),
                clap::arg!(--arch <arch> "Target architecture")
                    .value_parser(clap::builder::EnumValueParser::<Arch>::new()),
                clap::arg!(--verbose "Print commands"),
            ]),
        )
        .subcommand(
            clap::Command::new("dist").about("Builds a multibootable r9 image").args(&[
                clap::arg!(--release "Build a release version").conflicts_with("debug"),
                clap::arg!(--debug "Build a debug version").conflicts_with("release"),
                clap::arg!(--arch <arch> "Target architecture")
                    .value_parser(clap::builder::EnumValueParser::<Arch>::new()),
                clap::arg!(--config <name> "Configuration")
                    .value_parser(clap::builder::NonEmptyStringValueParser::new())
                    .default_value("default"),
                clap::arg!(--verbose "Print commands"),
            ]),
        )
        .subcommand(clap::Command::new("test").about("Runs unit tests").args(&[
            clap::arg!(--release "Build a release version").conflicts_with("debug"),
            clap::arg!(--debug "Build a debug version").conflicts_with("release"),
            clap::arg!(--json "Output messages as json"),
            clap::arg!(--verbose "Print commands"),
        ]))
        .subcommand(
            clap::Command::new("clippy").about("Runs clippy").args(&[
                clap::arg!(--release "Build a release version").conflicts_with("debug"),
                clap::arg!(--debug "Build a debug version").conflicts_with("release"),
                clap::arg!(--arch <arch> "Target architecture")
                    .value_parser(clap::builder::EnumValueParser::<Arch>::new()),
                clap::arg!(--config <name> "Configuration")
                    .value_parser(clap::builder::NonEmptyStringValueParser::new())
                    .default_value("default"),
                clap::arg!(--verbose "Print commands"),
                clap::arg!(--semihosting "Lint the kernel with its semihosting feature, as smoke --semihosting builds it"),
            ]),
        )
        .subcommand(clap::Command::new("check").about("Runs check").args(&[
            clap::arg!(--json "Output messages as json"),
            clap::arg!(--verbose "Print commands"),
        ]))
        .subcommand(
            clap::Command::new("qemu").about("Run r9 under QEMU").args(&[
                clap::arg!(--release "Build a release version").conflicts_with("debug"),
                clap::arg!(--debug "Build a debug version").conflicts_with("release"),
                clap::arg!(--arch <arch> "Target architecture")
                    .value_parser(clap::builder::EnumValueParser::<Arch>::new()),
                clap::arg!(--gdb "Wait for gdb connection on start"),
                clap::arg!(--kvm "Run with KVM"),
                clap::arg!(--config <name> "Configuration")
                    .value_parser(clap::builder::NonEmptyStringValueParser::new())
                    .default_value("default"),
                clap::arg!(--verbose "Print commands"),
                clap::arg!(--dump_dtb <file> "Dump the DTB from QEMU to a file")
                    .value_parser(clap::value_parser!(String)),
            ]),
        )
        .subcommand(
            clap::Command::new("smoke")
                .about("Boot r9 under QEMU, type at its console, and check that the user programs run")
                .args(&[
                    clap::arg!(--release "Build a release version").conflicts_with("debug"),
                    clap::arg!(--debug "Build a debug version").conflicts_with("release"),
                    clap::arg!(--arch <arch> "Target architecture")
                        .value_parser(clap::builder::EnumValueParser::<Arch>::new()),
                    clap::arg!(--config <name> "Configuration")
                        .value_parser(clap::builder::NonEmptyStringValueParser::new())
                        .default_value("default"),
                    clap::arg!(--verbose "Print commands"),
                    clap::arg!(
                        --semihosting "Build with QEMU semihosting, and take QEMU's exit status as the result.  The image this leaves in target/ is for QEMU only: on hardware, semihosting is an undefined instruction"
                    ),
                ]),
        )
        .subcommand(clap::Command::new("clean").about("Cargo clean"))
        .get_matches();

    if let Err(e) = match matches.subcommand() {
        Some(("build", m)) => BuildStep::new(m).run(),
        Some(("expand", m)) => ExpandStep::new(m).run(),
        Some(("kasm", m)) => KasmStep::new(m).run(),
        Some(("dist", m)) => {
            let s1 = BuildStep::new(m);
            let s2 = DistStep::new(m);
            s1.run().and_then(|_| s2.run())
        }
        Some(("test", m)) => TestStep::new(m).run(),
        Some(("clippy", m)) => ClippyStep::new(m).run(),
        Some(("check", m)) => CheckStep::new(m).run(),
        Some(("qemu", m)) => {
            let s1 = BuildStep::new(m);
            let s2 = DistStep::new(m);
            let s3 = QemuStep::new(m);
            s1.run().and_then(|_| s2.run()).and_then(|_| s3.run())
        }
        Some(("smoke", m)) => {
            let s1 = BuildStep::new(m);
            let s2 = DistStep::new(m);
            let s3 = SmokeStep::new(m);
            s1.run().and_then(|_| s2.run()).and_then(|_| s3.run())
        }
        Some(("clean", _)) => CleanStep::new().run(),
        _ => Err("bad subcommand".into()),
    } {
        eprintln!("{e}");
        process::exit(1);
    }
}

fn env_or(var: &str, default: &str) -> String {
    let default = default.to_string();
    env::var(var).unwrap_or(default)
}

fn cargo() -> String {
    env_or("CARGO", "cargo")
}

fn objcopy() -> String {
    let toolchain = env_or("RUSTUP_TOOLCHAIN", "nightly-x86_64-unknown-none");
    let home = env_or("RUSTUP_HOME", "");
    let segments: Vec<&str> = toolchain.split('-').collect();

    // The toolchain dir is named `<channel>-<host triple>`, and the triple
    // has 3 or 4 dash-separated parts (aarch64-apple-darwin,
    // x86_64-unknown-linux-gnu), so try both lengths.  A name with fewer
    // parts, such as a bare channel, matches no dir and falls back to PATH.
    let llvm_objcopy = (3..=4)
        .find_map(|n| {
            let host = segments[segments.len().saturating_sub(n)..].join("-");
            let mut path = PathBuf::from(&home);
            path.push("toolchains");
            path.push(&toolchain);
            path.push("lib");
            path.push("rustlib");
            path.push(host);
            path.push("bin");
            path.push("llvm-objcopy");
            path.exists().then(|| path.into_os_string().into_string().unwrap())
        })
        .unwrap_or_else(|| "llvm-objcopy".to_string());

    env_or("OBJCOPY", &llvm_objcopy)
}

fn load_config(arch: Arch, matches: &clap::ArgMatches) -> Configuration {
    let default = "default".to_string();
    let config_file = matches.try_get_one("config").ok().flatten().unwrap_or(&default);
    Configuration::load(format!(
        "{}/{}/lib/config_{}.toml",
        workspace().display(),
        arch.to_string().to_lowercase(),
        config_file
    ))
}

fn verbose(matches: &clap::ArgMatches) -> bool {
    matches.get_flag("verbose")
}

/// Whether to build, lint or run with QEMU semihosting.  Only clippy and smoke
/// have the flag.
fn semihosting(matches: &clap::ArgMatches) -> bool {
    matches.try_get_one::<bool>("semihosting").ok().flatten().copied().unwrap_or(false)
}

struct BuildStep {
    arch: Arch,
    config: Configuration,
    profile: Profile,
    semihosting: bool,
    verbose: bool,
}

impl BuildStep {
    fn new(matches: &clap::ArgMatches) -> Self {
        let arch = Arch::from(matches);
        let config = load_config(arch, matches);
        let profile = Profile::from(matches);
        let semihosting = semihosting(matches);
        let verbose = verbose(matches);

        Self { arch, config, profile, semihosting, verbose }
    }

    fn run(self) -> Result<()> {
        let user_bin = build_user(self.arch, self.profile, self.verbose)?;

        let mut cmd = Command::new(cargo());
        cmd.arg("build");
        set_user_bin(&mut cmd, &user_bin);

        apply_to_build_step(
            &mut cmd,
            &self.config,
            &self.arch.target(),
            &self.profile,
            workspace().to_str().unwrap(),
        );

        cmd.current_dir(workspace());
        cmd.arg("--workspace");
        cmd.arg("--exclude").arg("xtask");
        // The user crates are built by build_user: the kernel's target spec
        // and linker script aren't for them.
        exclude_user(&mut cmd);
        exclude_other_arches(self.arch, &mut cmd);
        if self.semihosting {
            cmd.arg("--features").arg("aarch64/semihosting");
        }
        if self.profile == Profile::Release {
            cmd.arg("--release");
        }
        cmd.arg("-Z").arg("build-std=core,alloc");
        cmd.arg("-Z").arg("json-target-spec");
        if self.verbose {
            println!("Executing {cmd:?}");
        }
        let status = annotated_status(&mut cmd)?;
        if !status.success() {
            return Err("build kernel failed".into());
        }
        Ok(())
    }
}

struct DistStep {
    arch: Arch,
    profile: Profile,
    verbose: bool,
}

impl DistStep {
    fn new(matches: &clap::ArgMatches) -> Self {
        let arch = Arch::from(matches);
        let profile = Profile::from(matches);
        let verbose = verbose(matches);
        Self { arch, profile, verbose }
    }

    fn run(self) -> Result<()> {
        match self.arch {
            Arch::Aarch64 => {
                // Qemu needs a flat binary in order to handle device tree files correctly
                let mut cmd = Command::new(objcopy());
                cmd.arg("-O");
                cmd.arg("binary");
                cmd.arg(format!("target/{}/{}/aarch64", self.arch.target(), self.profile.dir()));
                cmd.arg(format!(
                    "target/{}/{}/aarch64-qemu",
                    self.arch.target(),
                    self.profile.dir()
                ));
                cmd.current_dir(workspace());
                if self.verbose {
                    println!("Executing {cmd:?}");
                }
                let status = annotated_status(&mut cmd)?;
                if !status.success() {
                    return Err("objcopy failed".into());
                }

                // Compress the binary.  We do this because they're much faster when used
                // for netbooting and qemu also accepts them.
                let mut cmd = Command::new("gzip");
                cmd.arg("-k");
                cmd.arg("-f");
                cmd.arg(format!(
                    "target/{}/{}/aarch64-qemu",
                    self.arch.target(),
                    self.profile.dir()
                ));
                cmd.current_dir(workspace());
                if self.verbose {
                    println!("Executing {cmd:?}");
                }
                let status = annotated_status(&mut cmd)?;
                if !status.success() {
                    return Err("gzip failed".into());
                }
            }
            Arch::X86_64 => {
                let mut cmd = Command::new(objcopy());
                cmd.arg("--input-target=elf64-x86-64");
                cmd.arg("--output-target=elf32-i386");
                cmd.arg(format!("target/{}/{}/x86_64", self.arch.target(), self.profile.dir()));
                cmd.arg(format!("target/{}/{}/r9.elf32", self.arch.target(), self.profile.dir()));
                cmd.current_dir(workspace());
                if self.verbose {
                    println!("Executing {cmd:?}");
                }
                let status = annotated_status(&mut cmd)?;
                if !status.success() {
                    return Err("objcopy failed".into());
                }
            }
            Arch::Riscv64 => {
                // Qemu needs a flat binary in order to handle device tree files correctly
                let mut cmd = Command::new(objcopy());
                cmd.arg("-O");
                cmd.arg("binary");
                cmd.arg(format!("target/{}/{}/riscv64", self.arch.target(), self.profile.dir()));
                cmd.arg(format!(
                    "target/{}/{}/riscv64-qemu",
                    self.arch.target(),
                    self.profile.dir()
                ));
                cmd.current_dir(workspace());
                if self.verbose {
                    println!("Executing {cmd:?}");
                }
                let status = annotated_status(&mut cmd)?;
                if !status.success() {
                    return Err("objcopy failed".into());
                }
            }
        };

        Ok(())
    }
}

struct QemuStep {
    arch: Arch,
    config: Configuration,
    profile: Profile,
    wait_for_gdb: bool,
    kvm: bool,
    dump_dtb: String,
    semihosting: bool,
    verbose: bool,
}

impl QemuStep {
    fn new(matches: &clap::ArgMatches) -> Self {
        let arch = Arch::from(matches);
        let config = load_config(arch, matches);
        let profile = Profile::from(matches);
        let wait_for_gdb = matches.get_flag("gdb");
        let kvm = matches.get_flag("kvm");
        let dump_dtb: String = matches
            .try_get_one::<String>("dump_dtb")
            .ok()
            .flatten()
            .unwrap_or(&"".to_string())
            .clone();
        let verbose = verbose(matches);

        Self { arch, config, profile, wait_for_gdb, kvm, dump_dtb, semihosting: false, verbose }
    }

    fn run(self) -> Result<()> {
        let mut cmd = self.command()?;
        let status = annotated_status(&mut cmd)?;
        if !status.success() {
            return Err("qemu failed".into());
        }
        Ok(())
    }

    /// The QEMU command to run r9 with, with its serial console on stdio.
    fn command(&self) -> Result<Command> {
        let target = self.arch.target();
        let dir = self.profile.dir();
        let qemu_system = self.arch.qemu_system();

        if self.kvm && self.arch != Arch::X86_64 {
            return Err("KVM only supported under x86-64".into());
        }

        let cmd = match self.arch {
            Arch::Aarch64 => {
                let mut cmd = Command::new(qemu_system);

                apply_to_qemu_step(&mut cmd, &self.config);

                // TODO Choose UART at cmdline
                // If using UART0 (PL011), this enables serial
                cmd.arg("-nographic");

                // If using UART1 (MiniUART), this enables serial
                cmd.arg("-serial");
                cmd.arg("null");
                cmd.arg("-serial");
                cmd.arg("mon:stdio");

                if self.wait_for_gdb {
                    cmd.arg("-s").arg("-S");
                }
                cmd.arg("-kernel");
                cmd.arg(format!("target/{target}/{dir}/aarch64-qemu.gz"));
                if self.semihosting {
                    cmd.arg("-semihosting-config").arg("enable=on,target=native");
                }
                cmd.current_dir(workspace());
                if self.verbose {
                    // Show exception level change events in stdout
                    cmd.arg("-d");
                    cmd.arg("int");

                    println!("Executing {cmd:?}");
                }
                cmd
            }
            Arch::Riscv64 => {
                let mut cmd = Command::new(qemu_system);
                cmd.arg("-nographic");
                //cmd.arg("-curses");
                // cmd.arg("-bios").arg("none");
                let dump_dtb = &self.dump_dtb;
                if !dump_dtb.is_empty() {
                    cmd.arg("-machine").arg(format!("virt,dumpdtb={dump_dtb}"));
                } else {
                    cmd.arg("-machine").arg("virt");
                }
                cmd.arg("-cpu").arg("rv64");
                // FIXME: This is not needed as of now, and will only work once the
                // FIXME: disk.bin is also taken care of. Doesn't exist by default.
                if false {
                    cmd.arg("-drive").arg("file=disk.bin,format=raw,id=hd0");
                    cmd.arg("-device").arg("virtio-blk-device,drive=hd0");
                }
                cmd.arg("-netdev").arg("type=user,id=net0");
                cmd.arg("-device").arg("virtio-net-device,netdev=net0");
                cmd.arg("-smp").arg("4");
                cmd.arg("-m").arg("1024M");
                cmd.arg("-serial").arg("mon:stdio");
                if self.wait_for_gdb {
                    cmd.arg("-s").arg("-S");
                }
                cmd.arg("-d").arg("guest_errors,unimp");
                cmd.arg("-kernel");
                cmd.arg(format!("target/{target}/{dir}/riscv64"));
                cmd.current_dir(workspace());
                if self.verbose {
                    println!("Executing {cmd:?}");
                }
                cmd
            }
            Arch::X86_64 => {
                let mut cmd = Command::new(qemu_system);
                cmd.arg("-nographic");
                // cmd.arg("-display");
                // cmd.arg("curses");
                if self.kvm {
                    cmd.arg("-accel").arg("kvm");
                    cmd.arg("-cpu").arg("host,pdpe1gb,xsaveopt,fsgsbase,apic,msr");
                } else {
                    cmd.arg("-M").arg("q35");
                    cmd.arg("-cpu").arg("qemu64,pdpe1gb,xsaveopt,fsgsbase,apic,msr");
                }
                cmd.arg("-smp");
                cmd.arg("8");
                cmd.arg("-s");
                cmd.arg("-m");
                cmd.arg("8192");
                if self.wait_for_gdb {
                    cmd.arg("-s").arg("-S");
                }
                //cmd.arg("-device");
                //cmd.arg("ahci,id=ahci0");
                //cmd.arg("-drive");
                //cmd.arg("id=sdahci0,file=sdahci0.img,if=none");
                //cmd.arg("-device");
                //cmd.arg("ide-hd,drive=sdahci0,bus=ahci0.0");
                cmd.arg("-kernel");
                cmd.arg(format!("target/{target}/{dir}/r9.elf32"));
                cmd.current_dir(workspace());
                if self.verbose {
                    println!("Executing {cmd:?}");
                }
                cmd
            }
        };

        Ok(cmd)
    }
}

/// How long the smoke test waits for the user programs to exit.
const SMOKE_TIMEOUT: Duration = Duration::from_secs(60);

/// What the smoke test types at the shell, each at its next prompt, with a
/// return ending a line, as a terminal sends it: factor 21; cat, a line for
/// it, and ^D, cat's end of file; a command that doesn't exist; echo, so the
/// last command succeeds and so does the shell, which exits with its
/// status; and ^D, the shell's end of file.
const SHELL_INPUT: [&[u8]; 5] = [
    b"factor 21\r",
    // At once, as the console's read returns at most a line: the shell reads
    // "cat", and cat the rest, each line echoed as it arrives, before cat
    // prints it.
    b"cat\rtyped at the console\r\x04",
    b"nosuch\r",
    b"echo typed at the shell\r",
    b"\x04",
];

/// What the shell's session prints, prompts and the console's echo of what's
/// typed included, ^D's echo dropped: factor's number, its factors and a
/// blank line; the line typed for cat, echoed, then printed; rc's error for
/// the missing command; echo's words; and the kernel's report of the shell's
/// exit after its last prompt.
const SHELL_OUTPUT: [&str; 13] = [
    "% factor 21",
    "21",
    "     3",
    "     7",
    "",
    "% cat",
    "typed at the console",
    "typed at the console",
    "% nosuch",
    "nosuch: '/bin/nosuch' file does not exist",
    "% echo typed at the shell",
    "typed at the shell",
    "% sh: exited",
];

/// The shell's prompt, at the start of a line.
const PROMPT: &str = "% ";

/// A line of QEMU's output, as the smoke test checks it: lossily, so a byte
/// that isn't UTF-8 doesn't end the capture, without its return, and without
/// the ^Ds the console echoes as they're typed, which a terminal doesn't
/// show.
fn smoke_line(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).trim_end_matches('\r').replace('\x04', "")
}

/// The four numbers of `line`, if it's cat's output of /dev/time: Plan 9's
/// time text, each number right-aligned in its width and followed by a
/// space, and no newline, so the kernel's "cat: exited" follows it on its
/// line.
fn time_line(line: &str) -> Option<Vec<u64>> {
    let text = line.strip_suffix("cat: exited")?;
    let fields: Vec<u64> = text.split_whitespace().filter_map(|f| f.parse().ok()).collect();
    (text.len() == 78
        && [11, 33, 55, 77].iter().all(|&i| text.as_bytes()[i] == b' ')
        && fields.len() == 4)
        .then_some(fields)
}

/// Boot r9 under QEMU, and check that the user programs run: factor's output
/// must be exactly right, sleep and factor must both exit, cat must print the
/// time, echo must run after, and the shell last, printing exactly what
/// SHELL_OUTPUT says for what's typed at it, all with no panic.  QEMU doesn't
/// exit by itself, so it's killed once the shell has exited, a program has
/// failed, the kernel has panicked, or SMOKE_TIMEOUT has passed.  With
/// semihosting, the kernel makes QEMU exit, with status 0 only if all
/// succeeded, and that status must be 0 too.
struct SmokeStep {
    qemu: QemuStep,
}

impl SmokeStep {
    fn new(matches: &clap::ArgMatches) -> Self {
        let arch = Arch::from(matches);
        let qemu = QemuStep {
            arch,
            config: load_config(arch, matches),
            profile: Profile::from(matches),
            wait_for_gdb: false,
            kvm: false,
            dump_dtb: String::new(),
            semihosting: semihosting(matches),
            verbose: verbose(matches),
        };
        Self { qemu }
    }

    fn run(self) -> Result<()> {
        if self.qemu.arch != Arch::Aarch64 {
            return Err("smoke test: only aarch64 runs user programs".into());
        }
        let mut cmd = self.qemu.command()?;
        cmd.stdin(Stdio::piped()).stdout(Stdio::piped());
        let mut child = cmd.spawn().map_err(|e| format!("{}: {e}", cmd.get_program().display()))?;
        let stdout = child.stdout.take().ok_or("smoke test: no stdout from QEMU")?;
        let mut stdin = child.stdin.take().ok_or("smoke test: no stdin to QEMU")?;

        // Read QEMU's output on another thread, so the wait can time out.
        // It's sent as it comes, not a line at a time, as the shell's prompt
        // ends without a newline.
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let mut r = stdout;
            let mut buf = [0; 4096];
            while let Ok(n @ 1..) = r.read(&mut buf) {
                if tx.send(buf[..n].to_vec()).is_err() {
                    break;
                }
            }
        });

        let deadline = Instant::now() + SMOKE_TIMEOUT;
        let mut lines: Vec<String> = Vec::new();
        // What's come since the last newline.
        let mut partial: Vec<u8> = Vec::new();
        let mut timed_out = false;
        let mut exited = false;
        let mut typed = 0;
        loop {
            match rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
                Ok(bytes) => {
                    partial.extend_from_slice(&bytes);
                    let mut done = false;
                    while let Some(i) = partial.iter().position(|&b| b == b'\n') {
                        let line = smoke_line(&partial[..i]);
                        partial.drain(..=i);
                        // The shell has exited, successfully or not, a
                        // program has failed, which stops the rest running,
                        // or the kernel has panicked: there's nothing more to
                        // wait for, unless the kernel will make QEMU exit.
                        done |= !self.qemu.semihosting
                            && (line.contains("sh: exited")
                                || line.contains(": exited with status")
                                || line.contains("panicked"));
                        lines.push(line);
                    }
                    // Type at each prompt the shell prints, once it's printed,
                    // so the console's echo follows it.  Errors are ignored:
                    // QEMU may have exited, which is reported below.
                    let prompts = lines.iter().filter(|l| l.starts_with(PROMPT)).count()
                        + usize::from(smoke_line(&partial).starts_with(PROMPT));
                    if typed < SHELL_INPUT.len() && prompts > typed {
                        let _ = stdin.write_all(SHELL_INPUT[typed]).and_then(|_| stdin.flush());
                        typed += 1;
                    }
                    if done {
                        break;
                    }
                }
                Err(RecvTimeoutError::Timeout) => {
                    timed_out = true;
                    break;
                }
                // QEMU exited.
                Err(RecvTimeoutError::Disconnected) => {
                    exited = true;
                    break;
                }
            }
        }
        // Errors are ignored: QEMU may already have exited.
        let _ = child.kill();
        let status = child.wait();

        // Whether `want` is an exact line, and where, so the order of the
        // exit lines can be checked too.
        let has = |want: &str| lines.iter().any(|l| l == want);
        let at = |want: &str| lines.iter().position(|l| l == want);
        let before = |a: &str, b: &str| match (at(a), at(b)) {
            (Some(x), Some(y)) => x < y,
            _ => false,
        };
        // factor's whole output, in order: its argument, its factors, and the
        // blank line it prints after them.
        let factor_exact = has("9007195909437503")
            && has("     94906247")
            && has("     94906249")
            && at("     94906249").is_some_and(|i| lines.get(i + 1).is_some_and(|l| l.is_empty()));
        // cat /dev/time's output, whose last number is the counter's
        // frequency, which the kernel prints at boot.
        let hz = lines.iter().find_map(|l| {
            l.strip_prefix("Counter frequency: ")?.strip_suffix(" Hz")?.parse::<u64>().ok()
        });
        let time_at = lines.iter().position(|l| time_line(l).is_some_and(|f| Some(f[3]) == hz));
        // Then the shell's session, exactly.
        let shell_at = lines.windows(SHELL_OUTPUT.len()).position(|w| w == SHELL_OUTPUT);
        let qemu_exited = |status: &std::io::Result<process::ExitStatus>| match status {
            Ok(s) => format!("QEMU exited with {s}"),
            Err(e) => format!("QEMU exited: {e}"),
        };
        let failure = if exited && !self.qemu.semihosting {
            Some(qemu_exited(&status))
        } else if lines.iter().any(|l| l.contains("panicked")) {
            Some("the kernel panicked".to_string())
        } else if self.qemu.semihosting && timed_out {
            Some("timed out before QEMU exited".to_string())
        } else if self.qemu.semihosting && !matches!(&status, Ok(s) if s.success()) {
            Some(qemu_exited(&status))
        } else if !factor_exact {
            Some("factor's output isn't exactly right".to_string())
        } else if !has("factor: exited") {
            Some("factor didn't exit successfully".to_string())
        } else if !has("sleep: exited") {
            Some("sleep didn't exit successfully".to_string())
        } else if time_at.is_none() {
            Some("cat didn't print the time, or didn't exit successfully".to_string())
        } else if !has("hello world") {
            Some("echo didn't print \"hello world\"".to_string())
        } else if !has("echo: exited") {
            Some(if timed_out {
                "timed out before echo exited".to_string()
            } else {
                "echo didn't exit successfully".to_string()
            })
        } else if !before("factor: exited", "echo: exited") {
            Some("echo didn't run after factor".to_string())
        } else if shell_at.is_none() {
            Some(if timed_out {
                "timed out before the shell exited".to_string()
            } else {
                "the shell's session isn't exactly right".to_string()
            })
        } else if !before("echo: exited", SHELL_OUTPUT[0]) {
            Some("the shell didn't run after echo".to_string())
        } else {
            None
        };
        match failure {
            None => {
                println!("smoke test: ok");
                Ok(())
            }
            Some(why) => {
                for l in &lines {
                    println!("{l}");
                }
                Err(format!("smoke test failed: {why}").into())
            }
        }
    }
}

struct ExpandStep {
    arch: Arch,
    profile: Profile,
    verbose: bool,
}

impl ExpandStep {
    fn new(matches: &clap::ArgMatches) -> Self {
        let arch = Arch::from(matches);
        let profile = Profile::from(matches);
        let verbose = verbose(matches);

        Self { arch, profile, verbose }
    }

    fn run(self) -> Result<()> {
        let user_bin = build_user(self.arch, self.profile, self.verbose)?;
        let mut cmd = Command::new(cargo());
        cmd.current_dir(workspace());
        set_user_bin(&mut cmd, &user_bin);
        cmd.arg("rustc");
        cmd.arg("-Z").arg("build-std=core,alloc");
        cmd.arg("-Z").arg("json-target-spec");
        cmd.arg("-p").arg(self.arch.to_string().to_lowercase());
        cmd.arg("--target").arg(format!("lib/{}.json", self.arch.target()));
        cmd.arg("--");
        cmd.arg("-Z").arg("unpretty=expanded");
        if self.profile == Profile::Release {
            cmd.arg("--release");
        }
        if self.verbose {
            println!("Executing {cmd:?}");
        }
        let status = annotated_status(&mut cmd)?;
        if !status.success() {
            return Err("build kernel failed".into());
        }
        Ok(())
    }
}

struct KasmStep {
    arch: Arch,
    profile: Profile,
    verbose: bool,
}

impl KasmStep {
    fn new(matches: &clap::ArgMatches) -> Self {
        let arch = Arch::from(matches);
        let profile = Profile::from(matches);
        let verbose = verbose(matches);

        Self { arch, profile, verbose }
    }

    fn run(self) -> Result<()> {
        let user_bin = build_user(self.arch, self.profile, self.verbose)?;
        let mut cmd = Command::new(cargo());
        cmd.current_dir(workspace());
        set_user_bin(&mut cmd, &user_bin);
        cmd.arg("rustc");
        cmd.arg("-Z").arg("build-std=core,alloc");
        cmd.arg("-Z").arg("json-target-spec");
        cmd.arg("-p").arg(self.arch.to_string().to_lowercase());
        cmd.arg("--target").arg(format!("lib/{}.json", self.arch.target()));
        cmd.arg("--").arg("--emit").arg("asm");
        if self.profile == Profile::Release {
            cmd.arg("--release");
        }
        if self.verbose {
            println!("Executing {cmd:?}");
        }
        let status = annotated_status(&mut cmd)?;
        if !status.success() {
            return Err("build kernel failed".into());
        }
        Ok(())
    }
}

/// Run tests for the current host toolchain.
struct TestStep {
    json_output: bool,
    verbose: bool,
}

impl TestStep {
    fn new(matches: &clap::ArgMatches) -> Self {
        let json_output = matches.get_flag("json");
        let verbose = verbose(matches);

        Self { json_output, verbose }
    }

    fn run(self) -> Result<()> {
        // port's ELF tests, and the aarch64 kernel, embed user programs.
        let user_bin = build_user(Arch::Aarch64, Profile::Debug, self.verbose)?;

        let mut all_cmd_args = Vec::new();

        all_cmd_args.push(vec![
            "test".to_string(),
            "--package".to_string(),
            "port".to_string(),
            "--lib".to_string(),
        ]);

        let rustup_state = RustupState::new();

        let arch = std::env::consts::ARCH;
        if let Some(target) = rustup_state.std_supported_target(arch) {
            all_cmd_args.push(vec![
                "test".to_string(),
                "--package".to_string(),
                arch.to_string(),
                "--bins".to_string(),
                "--target".to_string(),
                target.to_string(),
            ]);
            // echo's logic is a library, tested on the host.
            all_cmd_args.push(vec![
                "test".to_string(),
                "--package".to_string(),
                "echo".to_string(),
                "--lib".to_string(),
                "--target".to_string(),
                target.to_string(),
            ]);
            // sleep's argument parsing is a library, tested on the host.
            all_cmd_args.push(vec![
                "test".to_string(),
                "--package".to_string(),
                "sleep".to_string(),
                "--lib".to_string(),
                "--target".to_string(),
                target.to_string(),
            ]);
            // factor's factorization is a library, tested on the host.
            all_cmd_args.push(vec![
                "test".to_string(),
                "--package".to_string(),
                "factor".to_string(),
                "--lib".to_string(),
                "--target".to_string(),
                target.to_string(),
            ]);
            // The shell's word splitting and program search are a library,
            // tested on the host.
            all_cmd_args.push(vec![
                "test".to_string(),
                "--package".to_string(),
                "sh".to_string(),
                "--lib".to_string(),
                "--target".to_string(),
                target.to_string(),
            ]);
            // The user library's parsing and math are tested on the host.
            all_cmd_args.push(vec![
                "test".to_string(),
                "--package".to_string(),
                "r9-libc".to_string(),
                "--lib".to_string(),
                "--target".to_string(),
                target.to_string(),
            ]);
        }

        for cmd_args in all_cmd_args {
            let mut cmd = Command::new(cargo());
            cmd.current_dir(workspace());
            set_user_bin(&mut cmd, &user_bin);

            cmd.args(cmd_args);
            if self.json_output {
                cmd.arg("--message-format=json").arg("--quiet");
            }

            if self.verbose {
                println!("Executing {cmd:?}");
            }
            let status = annotated_status(&mut cmd)?;
            if !status.success() {
                return Err("test failed".into());
            }
        }
        Ok(())
    }
}

struct ClippyStep {
    arch: Arch,
    config: Configuration,
    profile: Profile,
    semihosting: bool,
    verbose: bool,
}

impl ClippyStep {
    fn new(matches: &clap::ArgMatches) -> Self {
        let arch = Arch::from(matches);
        let config = load_config(arch, matches);
        let profile = Profile::from(matches);
        let semihosting = semihosting(matches);
        let verbose = verbose(matches);

        Self { arch, config, profile, semihosting, verbose }
    }

    fn run(self) -> Result<()> {
        if self.semihosting && self.arch != Arch::Aarch64 {
            return Err("clippy: only aarch64 has semihosting".into());
        }

        // The aarch64 kernel embeds user programs, so they're built first.
        let user_bin = build_user(self.arch, self.profile, self.verbose)?;

        let mut cmd = Command::new(cargo());
        cmd.arg("clippy");
        set_user_bin(&mut cmd, &user_bin);

        apply_to_clippy_step(&mut cmd, &self.config);

        cmd.current_dir(workspace());
        cmd.arg("--workspace");
        exclude_other_arches(self.arch, &mut cmd);
        // The user crates are aarch64 programs, linted with the aarch64 kernel.
        if self.arch != Arch::Aarch64 {
            exclude_user(&mut cmd);
        }
        if self.semihosting {
            cmd.arg("--features").arg("aarch64/semihosting");
        }
        if self.profile == Profile::Release {
            cmd.arg("--release");
        }
        cmd.arg("--").arg("-Dwarnings");
        if self.verbose {
            println!("Executing {cmd:?}");
        }
        let status = annotated_status(&mut cmd)?;
        if !status.success() {
            return Err("clippy failed".into());
        }
        Ok(())
    }
}

/// Run check for all packages for all relevant toolchains.
/// This assumes that the <arch>-unknown-linux-gnu toolchain has been installed
/// for any arch we care about.
struct CheckStep {
    json_output: bool,
    verbose: bool,
}

impl CheckStep {
    fn new(matches: &clap::ArgMatches) -> Self {
        let json_output = matches.get_flag("json");
        let verbose = verbose(matches);

        Self { json_output, verbose }
    }

    fn run(self) -> Result<()> {
        // To run check for bins and lib we use the default toolchain, which has
        // been set to the OS-independent arch toolchain in each Cargo.toml file.
        // The same applies to tests and benches for non-arch-specific lib packages.
        let bins_lib_package_cmd_args = vec![
            vec![
                "check".to_string(),
                "--package".to_string(),
                "aarch64".to_string(),
                "--bins".to_string(),
            ],
            vec![
                "check".to_string(),
                "--package".to_string(),
                "riscv64".to_string(),
                "--bins".to_string(),
            ],
            vec![
                "check".to_string(),
                "--package".to_string(),
                "x86_64".to_string(),
                "--bins".to_string(),
            ],
            vec![
                "check".to_string(),
                "--package".to_string(),
                "port".to_string(),
                "--lib".to_string(),
                "--tests".to_string(),
                "--benches".to_string(),
            ],
            vec![
                "check".to_string(),
                "--package".to_string(),
                "r9-sys".to_string(),
                "--lib".to_string(),
            ],
        ];
        let user_lib_cmd_args = USER_LIBS.map(|p| {
            vec!["check".to_string(), "--package".to_string(), p.to_string(), "--lib".to_string()]
        });
        let user_program_cmd_args = USER_PROGRAMS.map(|p| {
            vec!["check".to_string(), "--package".to_string(), p.to_string(), "--bins".to_string()]
        });

        // port's ELF tests, and the aarch64 kernel, embed user programs.
        let user_bin = build_user(Arch::Aarch64, Profile::Debug, self.verbose)?;

        let rustup_state = RustupState::new();

        // However, running check for tests and benches in arch packages requires
        // that we use a toolchain with `std`, so we need an OS-specific toolchain.
        // If the arch matches that of the current toolchain, then that will be used
        // for check.  Otherwise we'll always default to <arch>-unknown-linux-gnu.
        let mut benches_tests_package_cmd_args = Vec::new();

        for arch in ["aarch64", "riscv64", "x86_64"] {
            let Some(target) = rustup_state.std_supported_target(arch) else {
                continue;
            };

            benches_tests_package_cmd_args.push(vec![
                "check".to_string(),
                "--package".to_string(),
                arch.to_string(),
                "--tests".to_string(),
                "--benches".to_string(),
                "--target".to_string(),
                target.to_string(),
            ]);
        }

        let all_cmd_args = [
            bins_lib_package_cmd_args,
            user_lib_cmd_args.to_vec(),
            user_program_cmd_args.to_vec(),
            benches_tests_package_cmd_args,
        ];
        for cmd_args in all_cmd_args.concat() {
            let mut cmd = Command::new(cargo());
            set_user_bin(&mut cmd, &user_bin);
            cmd.args(cmd_args);
            if self.json_output {
                cmd.arg("--message-format=json").arg("--quiet");
            }
            cmd.current_dir(workspace());

            if self.verbose {
                println!("Executing {cmd:?}");
            }
            let status = annotated_status(&mut cmd)?;
            if !status.success() {
                return Err("check failed".into());
            }
        }
        Ok(())
    }
}

struct CleanStep {}

impl CleanStep {
    fn new() -> Self {
        Self {}
    }

    fn run(self) -> Result<()> {
        let mut cmd = Command::new(cargo());
        cmd.current_dir(workspace());
        cmd.arg("clean");
        let status = annotated_status(&mut cmd)?;
        if !status.success() {
            return Err("clean failed".into());
        }
        Ok(())
    }
}

fn workspace() -> PathBuf {
    Path::new(&env!("CARGO_MANIFEST_DIR")).ancestors().nth(1).unwrap().to_path_buf()
}

/// The user libraries and programs, which are aarch64-only.  The programs are
/// what the kernel and port's tests embed.
const USER_LIBS: [&str; 1] = ["r9-libc"];
const USER_PROGRAMS: [&str; 7] = ["cat", "echo", "elftest", "exectest", "factor", "sh", "sleep"];

/// The target user programs are built for.
const USER_TARGET: &str = "aarch64-unknown-none";

/// Build the user programs for `arch`, and return the directory they're in,
/// or None if `arch` has none.  This must run before anything that embeds
/// them is compiled, which finds them through R9_USER_BIN.
fn build_user(arch: Arch, profile: Profile, verbose: bool) -> Result<Option<PathBuf>> {
    if arch != Arch::Aarch64 {
        return Ok(None);
    }
    let ld = workspace().join("user/lib/user.ld");
    let mut cmd = Command::new(cargo());
    cmd.current_dir(workspace());
    cmd.arg("build");
    for p in USER_PROGRAMS {
        cmd.arg("--package").arg(p);
    }
    cmd.arg("--target").arg(USER_TARGET);
    if profile == Profile::Release {
        cmd.arg("--release");
    }
    // A static ET_EXEC image, laid out by the user linker script, with 4KiB
    // pages rather than lld's aarch64 default of 64KiB.
    cmd.arg("--config").arg(format!(
        "build.rustflags=['-C', 'relocation-model=static', '-C', 'link-arg=-T{}', '-C', \
         'link-arg=-zmax-page-size=4096']",
        ld.display()
    ));
    if verbose {
        println!("Executing {cmd:?}");
    }
    let status = annotated_status(&mut cmd)?;
    if !status.success() {
        return Err("build user programs failed".into());
    }
    Ok(Some(workspace().join("target").join(USER_TARGET).join(profile.dir())))
}

/// Tell `cmd`'s compiles where the user programs are.
fn set_user_bin(cmd: &mut Command, user_bin: &Option<PathBuf>) {
    if let Some(dir) = user_bin {
        cmd.env("R9_USER_BIN", dir);
    }
}

fn exclude_user(cmd: &mut Command) {
    for c in USER_LIBS.iter().chain(&USER_PROGRAMS) {
        cmd.arg("--exclude").arg(c);
    }
}

/// Exclude architectures other than the one being built
fn exclude_other_arches(arch: Arch, cmd: &mut Command) {
    match arch {
        Arch::Aarch64 => {
            cmd.arg("--exclude").arg("riscv64");
            cmd.arg("--exclude").arg("x86_64");
        }
        Arch::Riscv64 => {
            cmd.arg("--exclude").arg("aarch64");
            cmd.arg("--exclude").arg("x86_64");
        }
        Arch::X86_64 => {
            cmd.arg("--exclude").arg("aarch64");
            cmd.arg("--exclude").arg("riscv64");
        }
    }
}

/// Annotates the error result with the calling binary's name.
fn annotated_status(cmd: &mut Command) -> Result<process::ExitStatus> {
    Ok(cmd.status().map_err(|e| format!("{}: {}", cmd.get_program().to_string_lossy(), e))?)
}
