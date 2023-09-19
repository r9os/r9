use std::{
    env, fmt,
    path::{Path, PathBuf},
    process::{self, Command},
};

mod config;
use crate::config::{generate_args, read_config, Configuration};

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

struct BuildParams {
    arch: Arch,
    profile: Profile,
    verbose: bool,
    wait_for_gdb: bool,
    config: Configuration,
    dump_dtb: String,
}

impl BuildParams {
    fn new(matches: &clap::ArgMatches) -> Self {
        let profile = if matches.get_flag("release") { Profile::Release } else { Profile::Debug };
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

        Self { arch: *arch, profile, verbose, wait_for_gdb, dump_dtb, config }
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

fn test(build_params: &BuildParams) -> Result<()> {
    let mut cmd = Command::new(cargo());
    cmd.current_dir(workspace());
    cmd.arg("test");
    cmd.arg("--workspace");
    cmd.arg("--target").arg("x86_64-unknown-linux-gnu");
    build_params.add_build_arg(&mut cmd);
    if build_params.verbose {
        println!("Executing {cmd:?}");
    }
    let status = annotated_status(&mut cmd)?;
    if !status.success() {
        return Err("test failed".into());
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
        return Err("build kernel failed".into());
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
