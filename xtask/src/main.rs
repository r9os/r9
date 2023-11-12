use crate::config::{generate_args, read_config, Configuration};
use rustup_configurator::Triple;
use std::{
    env, fmt,
    path::{Path, PathBuf},
    process::{self, Command},
};

mod config;

type DynError = Box<dyn std::error::Error>;
type Result<T> = std::result::Result<T, DynError>;

#[derive(Clone, Copy, Debug)]
pub enum Profile {
    Debug,
    Release,
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

impl fmt::Display for Arch {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{self:?}")
    }
}

// TODO This is becoming a bag of random fields - maybe turn into an enum
struct BuildParams {
    arch: Arch,
    profile: Profile,
    verbose: bool,
    wait_for_gdb: bool,
    config: Configuration,
    dump_dtb: String,
    json_output: bool,
}

impl BuildParams {
    fn new(matches: &clap::ArgMatches) -> Self {
        let profile = if matches.try_contains_id("release").unwrap_or(false) {
            Profile::Release
        } else {
            Profile::Debug
        };
        let verbose = matches.get_flag("verbose");
        let arch = matches.try_get_one("arch").ok().flatten().unwrap_or(&Arch::X86_64);
        let wait_for_gdb =
            matches.try_contains_id("gdb").unwrap_or(false) && matches.get_flag("gdb");

        let dump_dtb: String = matches
            .try_get_one::<String>("dump_dtb")
            .ok()
            .flatten()
            .unwrap_or(&"".to_string())
            .clone();
        let default = "default".to_string();
        let config_file = matches.try_get_one("config").ok().flatten().unwrap_or(&default);
        let config = read_config(format!(
            "{}/{}/lib/config_{}.toml",
            workspace().display(),
            arch.to_string().to_lowercase(),
            config_file
        ));

        // This is a very awkward way to check a boolean which may not exist...
        let json_output = matches.try_contains_id("json").unwrap_or(false)
            && matches.value_source("json") != Some(clap::parser::ValueSource::DefaultValue);

        Self { arch: *arch, profile, verbose, wait_for_gdb, dump_dtb, config, json_output }
    }

    fn dir(&self) -> &'static str {
        match self.profile {
            Profile::Debug => "debug",
            Profile::Release => "release",
        }
    }

    fn add_build_arg(&self, cmd: &mut Command) {
        if let Profile::Release = self.profile {
            cmd.arg("--release");
        }
    }

    fn qemu_system(&self) -> String {
        let defaultqemu = match self.arch {
            Arch::Aarch64 => "qemu-system-aarch64",
            Arch::Riscv64 => "qemu-system-riscv64",
            Arch::X86_64 => "qemu-system-x86_64",
        };
        env_or("QEMU", defaultqemu)
    }

    fn target(&self) -> String {
        env_or(
            "TARGET",
            format!("{}-unknown-none-elf", self.arch.to_string().to_lowercase()).as_str(),
        )
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
            installed_targets: rustup_configurator::installed().unwrap(),
            curr_toolchain: env::var("RUSTUP_TOOLCHAIN").unwrap(),
        }
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
                clap::arg!(--config <name> "Configuration")
                    .value_parser(clap::builder::NonEmptyStringValueParser::new())
                    .default_value("default"),
                clap::arg!(--verbose "Print commands"),
                clap::arg!(--dump_dtb <file> "Dump the DTB from QEMU to a file")
                    .value_parser(clap::value_parser!(String)),
            ]),
        )
        .subcommand(
            clap::Command::new("qemukvm").about("Run r9 under QEMU with KVM").args(&[
                clap::arg!(--release "Build a release version").conflicts_with("debug"),
                clap::arg!(--debug "Build a debug version").conflicts_with("release"),
                clap::arg!(--arch <arch> "Target architecture")
                    .value_parser(clap::builder::EnumValueParser::<Arch>::new()),
                clap::arg!(--gdb "Wait for gdb connection on start"),
                clap::arg!(--verbose "Print commands"),
                clap::arg!(--dump_dtb <file> "Dump the DTB from QEMU to a file")
                    .value_parser(clap::value_parser!(String)),
            ]),
        )
        .subcommand(clap::Command::new("clean").about("Cargo clean"))
        .get_matches();

    if let Err(e) = match matches.subcommand() {
        Some(("build", m)) => build(&BuildParams::new(m)),
        Some(("expand", m)) => expand(&BuildParams::new(m)),
        Some(("kasm", m)) => kasm(&BuildParams::new(m)),
        Some(("dist", m)) => dist(&BuildParams::new(m)),
        Some(("test", m)) => test(&BuildParams::new(m)),
        Some(("clippy", m)) => clippy(&BuildParams::new(m)),
        Some(("check", m)) => check(&BuildParams::new(m)),
        Some(("qemu", m)) => run(&BuildParams::new(m)),
        Some(("qemukvm", m)) => accelrun(&BuildParams::new(m)),
        Some(("clean", _)) => clean(),
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
    let llvm_objcopy = {
        let toolchain = env_or("RUSTUP_TOOLCHAIN", "nightly-x86_64-unknown-none");

        // find host architecture by taking last 3 segments from toolchain
        let mut arch_segments: Box<[_]> = toolchain.split('-').rev().take(3).collect();
        arch_segments.reverse();
        let host = arch_segments.join("-");

        let home = env_or("RUSTUP_HOME", "");
        let mut path = PathBuf::from(home);
        path.push("toolchains");
        path.push(toolchain);
        path.push("lib");
        path.push("rustlib");
        path.push(host);
        path.push("bin");
        path.push("llvm-objcopy");
        if path.exists() {
            path.into_os_string().into_string().unwrap()
        } else {
            "llvm-objcopy".into()
        }
    };
    env_or("OBJCOPY", &llvm_objcopy)
}

fn build(build_params: &BuildParams) -> Result<()> {
    let mut cmd = generate_args(
        "build",
        &build_params.config,
        &build_params.target(),
        &build_params.profile,
        workspace().to_str().unwrap(),
    );
    cmd.current_dir(workspace());
    cmd.arg("--workspace");
    cmd.arg("--exclude").arg("xtask");
    exclude_other_arches(build_params.arch, &mut cmd);
    build_params.add_build_arg(&mut cmd);
    if build_params.verbose {
        println!("Executing {cmd:?}");
    }
    let status = annotated_status(&mut cmd)?;
    if !status.success() {
        return Err("build kernel failed".into());
    }
    Ok(())
}

fn expand(build_params: &BuildParams) -> Result<()> {
    let mut cmd = Command::new(cargo());
    cmd.current_dir(workspace());
    cmd.arg("rustc");
    cmd.arg("-Z").arg("build-std=core,alloc");
    cmd.arg("-p").arg(build_params.arch.to_string().to_lowercase());
    cmd.arg("--target").arg(format!("lib/{}.json", build_params.target()));
    cmd.arg("--");
    cmd.arg("-Z").arg("unpretty=expanded");
    build_params.add_build_arg(&mut cmd);
    if build_params.verbose {
        println!("Executing {cmd:?}");
    }
    let status = annotated_status(&mut cmd)?;
    if !status.success() {
        return Err("build kernel failed".into());
    }
    Ok(())
}

fn kasm(build_params: &BuildParams) -> Result<()> {
    let mut cmd = Command::new(cargo());
    cmd.current_dir(workspace());
    cmd.arg("rustc");
    cmd.arg("-Z").arg("build-std=core,alloc");
    cmd.arg("-p").arg(build_params.arch.to_string().to_lowercase());
    cmd.arg("--target").arg(format!("lib/{}.json", build_params.target()));
    cmd.arg("--").arg("--emit").arg("asm");
    build_params.add_build_arg(&mut cmd);
    if build_params.verbose {
        println!("Executing {cmd:?}");
    }
    let status = annotated_status(&mut cmd)?;
    if !status.success() {
        return Err("build kernel failed".into());
    }
    Ok(())
}

fn dist(build_params: &BuildParams) -> Result<()> {
    build(build_params)?;

    match build_params.arch {
        Arch::Aarch64 => {
            // Qemu needs a flat binary in order to handle device tree files correctly
            let mut cmd = Command::new(objcopy());
            cmd.arg("-O");
            cmd.arg("binary");
            cmd.arg(format!("target/{}/{}/aarch64", build_params.target(), build_params.dir()));
            cmd.arg(format!(
                "target/{}/{}/aarch64-qemu",
                build_params.target(),
                build_params.dir()
            ));
            cmd.current_dir(workspace());
            if build_params.verbose {
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
                build_params.target(),
                build_params.dir()
            ));
            cmd.current_dir(workspace());
            if build_params.verbose {
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
            cmd.arg(format!("target/{}/{}/x86_64", build_params.target(), build_params.dir()));
            cmd.arg(format!("target/{}/{}/r9.elf32", build_params.target(), build_params.dir()));
            cmd.current_dir(workspace());
            if build_params.verbose {
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
            cmd.arg(format!("target/{}/{}/riscv64", build_params.target(), build_params.dir()));
            cmd.arg(format!(
                "target/{}/{}/riscv64-qemu",
                build_params.target(),
                build_params.dir()
            ));
            cmd.current_dir(workspace());
            if build_params.verbose {
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

/// Run tests for the current host toolchain.
fn test(build_params: &BuildParams) -> Result<()> {
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
    }

    for cmd_args in all_cmd_args {
        let mut cmd = Command::new(cargo());
        cmd.current_dir(workspace());

        cmd.args(cmd_args);
        if build_params.json_output {
            cmd.arg("--message-format=json").arg("--quiet");
        }

        if build_params.verbose {
            println!("Executing {cmd:?}");
        }
        let status = annotated_status(&mut cmd)?;
        if !status.success() {
            return Err("check failed".into());
        }
    }
    Ok(())
}

fn clippy(build_params: &BuildParams) -> Result<()> {
    let mut cmd = generate_args(
        "clippy",
        &build_params.config,
        &build_params.target(),
        &build_params.profile,
        workspace().to_str().unwrap(),
    );
    cmd.current_dir(workspace());
    cmd.arg("--workspace");
    exclude_other_arches(build_params.arch, &mut cmd);
    build_params.add_build_arg(&mut cmd);
    if build_params.verbose {
        println!("Executing {cmd:?}");
    }
    let status = annotated_status(&mut cmd)?;
    if !status.success() {
        return Err("clippy failed".into());
    }
    Ok(())
}

/// Run check for all packages for all relevant toolchains.
/// This assumes that the <arch>-unknown-linux-gnu toolchain has been installed
/// for any arch we care about.
fn check(build_params: &BuildParams) -> Result<()> {
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
    ];

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

    for cmd_args in [bins_lib_package_cmd_args, benches_tests_package_cmd_args].concat() {
        let mut cmd = Command::new(cargo());
        cmd.args(cmd_args);
        if build_params.json_output {
            cmd.arg("--message-format=json").arg("--quiet");
        }
        cmd.current_dir(workspace());

        if build_params.verbose {
            println!("Executing {cmd:?}");
        }
        let status = annotated_status(&mut cmd)?;
        if !status.success() {
            return Err("check failed".into());
        }
    }
    Ok(())
}

fn run(build_params: &BuildParams) -> Result<()> {
    dist(build_params)?;

    match build_params.arch {
        Arch::Aarch64 => {
            let mut cmd = Command::new(build_params.qemu_system());

            // TODO Choose UART at cmdline
            // If using UART0 (PL011), this enables serial
            cmd.arg("-nographic");

            // If using UART1 (MiniUART), this enables serial
            cmd.arg("-serial");
            cmd.arg("null");
            cmd.arg("-serial");
            cmd.arg("mon:stdio");

            cmd.arg("-M");
            cmd.arg("raspi3b");
            if build_params.wait_for_gdb {
                cmd.arg("-s").arg("-S");
            }
            cmd.arg("-dtb");
            cmd.arg("aarch64/lib/bcm2710-rpi-3-b.dtb");
            // Show exception level change events in stdout
            cmd.arg("-d");
            cmd.arg("int");
            cmd.arg("-kernel");
            cmd.arg(format!(
                "target/{}/{}/aarch64-qemu.gz",
                build_params.target(),
                build_params.dir()
            ));
            cmd.current_dir(workspace());
            if build_params.verbose {
                println!("Executing {cmd:?}");
            }
            let status = annotated_status(&mut cmd)?;
            if !status.success() {
                return Err("qemu failed".into());
            }
        }
        Arch::Riscv64 => {
            let mut cmd = Command::new(build_params.qemu_system());
            cmd.arg("-nographic");
            //cmd.arg("-curses");
            // cmd.arg("-bios").arg("none");
            let dump_dtb = &build_params.dump_dtb;
            if dump_dtb != "" {
                cmd.arg("-machine").arg(format!("virt,dumpdtb={dump_dtb}"));
            } else {
                cmd.arg("-machine").arg("virt");
            }
            cmd.arg("-cpu").arg("rv64");
            // FIXME: This is not needed as of now, and will only work once the
            // FIXME: // disk.bin is also taken care of. Doesn't exist by default.
            if false {
                cmd.arg("-drive").arg("file=disk.bin,format=raw,id=hd0");
                cmd.arg("-device").arg("virtio-blk-device,drive=hd0");
            }
            cmd.arg("-netdev").arg("type=user,id=net0");
            cmd.arg("-device").arg("virtio-net-device,netdev=net0");
            cmd.arg("-smp").arg("4");
            cmd.arg("-m").arg("1024M");
            cmd.arg("-serial").arg("mon:stdio");
            if build_params.wait_for_gdb {
                cmd.arg("-s").arg("-S");
            }
            cmd.arg("-d").arg("guest_errors,unimp");
            cmd.arg("-kernel");
            cmd.arg(format!("target/{}/{}/riscv64", build_params.target(), build_params.dir()));
            cmd.current_dir(workspace());
            if build_params.verbose {
                println!("Executing {cmd:?}");
            }
            let status = annotated_status(&mut cmd)?;
            if !status.success() {
                return Err("qemu failed".into());
            }
        }
        Arch::X86_64 => {
            let mut cmd = Command::new(build_params.qemu_system());
            cmd.arg("-nographic");
            //cmd.arg("-curses");
            cmd.arg("-M");
            cmd.arg("q35");
            cmd.arg("-cpu");
            cmd.arg("qemu64,pdpe1gb,xsaveopt,fsgsbase,apic,msr");
            cmd.arg("-smp");
            cmd.arg("8");
            cmd.arg("-s");
            cmd.arg("-m");
            cmd.arg("8192");
            if build_params.wait_for_gdb {
                cmd.arg("-s").arg("-S");
            }
            //cmd.arg("-device");
            //cmd.arg("ahci,id=ahci0");
            //cmd.arg("-drive");
            //cmd.arg("id=sdahci0,file=sdahci0.img,if=none");
            //cmd.arg("-device");
            //cmd.arg("ide-hd,drive=sdahci0,bus=ahci0.0");
            cmd.arg("-kernel");
            cmd.arg(format!("target/{}/{}/r9.elf32", build_params.target(), build_params.dir()));
            cmd.current_dir(workspace());
            if build_params.verbose {
                println!("Executing {cmd:?}");
            }
            let status = annotated_status(&mut cmd)?;
            if !status.success() {
                return Err("qemu failed".into());
            }
        }
    };

    Ok(())
}

fn accelrun(build_params: &BuildParams) -> Result<()> {
    dist(build_params)?;
    let mut cmd = Command::new(build_params.qemu_system());
    cmd.arg("-nographic");
    cmd.arg("-accel");
    cmd.arg("kvm");
    cmd.arg("-cpu");
    cmd.arg("host,pdpe1gb,xsaveopt,fsgsbase,apic,msr");
    cmd.arg("-smp");
    cmd.arg("8");
    cmd.arg("-m");
    cmd.arg("8192");
    if build_params.wait_for_gdb {
        cmd.arg("-s").arg("-S");
    }
    cmd.arg("-kernel");
    cmd.arg(format!("target/{}/{}/r9.elf32", build_params.target(), build_params.dir()));
    cmd.current_dir(workspace());
    if build_params.verbose {
        println!("Executing {cmd:?}");
    }
    let status = annotated_status(&mut cmd)?;
    if !status.success() {
        return Err("qemu failed".into());
    }
    Ok(())
}

fn clean() -> Result<()> {
    let mut cmd = Command::new(cargo());
    cmd.current_dir(workspace());
    cmd.arg("clean");
    let status = annotated_status(&mut cmd)?;
    if !status.success() {
        return Err("clean failed".into());
    }
    Ok(())
}

fn workspace() -> PathBuf {
    Path::new(&env!("CARGO_MANIFEST_DIR")).ancestors().nth(1).unwrap().to_path_buf()
}

// Exclude architectures other than the one being built
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

// Annotates the error result with the calling binary's name.
fn annotated_status(cmd: &mut Command) -> Result<process::ExitStatus> {
    Ok(cmd.status().map_err(|e| format!("{}: {}", cmd.get_program().to_string_lossy(), e))?)
}
