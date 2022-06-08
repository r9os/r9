use std::{
    env,
    path::{Path, PathBuf},
    process::{self, Command},
};

type DynError = Box<dyn std::error::Error>;
type Result<T> = std::result::Result<T, DynError>;

#[derive(Clone, Copy)]
enum Build {
    Debug,
    Release,
}

impl Build {
    fn dir(self) -> &'static str {
        match self {
            Self::Debug => "debug",
            Self::Release => "release",
        }
    }

    fn add_build_arg(self, cmd: &mut Command) {
        if let Self::Release = self {
            cmd.arg("--release");
        }
    }
}

fn main() {
    let matches = clap::Command::new("xtask")
        .version("0.1.0")
        .author("The r9 Authors")
        .about("Build support for the r9 operating system")
        .subcommand(clap::Command::new("build").about("Builds r9").args(&[
            clap::arg!(--release "Build release version").conflicts_with("debug"),
            clap::arg!(--debug "Build debug version (default)").conflicts_with("release"),
        ]))
        .subcommand(
            clap::Command::new("expand")
                .about("Expands r9 macros")
                .args(&[
                    clap::arg!(--release "Build release version").conflicts_with("debug"),
                    clap::arg!(--debug "Build debug version (default)").conflicts_with("release"),
                ]),
        )
        .subcommand(
            clap::Command::new("kasm")
                .about("Emits r9 assembler")
                .args(&[
                    clap::arg!(--release "Build release version").conflicts_with("debug"),
                    clap::arg!(--debug "Build debug version (default)").conflicts_with("release"),
                ]),
        )
        .subcommand(
            clap::Command::new("dist")
                .about("Builds a multibootable r9 image")
                .args(&[
                    clap::arg!(--release "Build a release version").conflicts_with("debug"),
                    clap::arg!(--debug "Build a debug version").conflicts_with("release"),
                ]),
        )
        .subcommand(clap::Command::new("test").about("Runs unit tests").args(&[
            clap::arg!(--release "Build a release version").conflicts_with("debug"),
            clap::arg!(--debug "Build a debug version").conflicts_with("release"),
        ]))
        .subcommand(clap::Command::new("clippy").about("Runs clippy").args(&[
            clap::arg!(--release "Build a release version").conflicts_with("debug"),
            clap::arg!(--debug "Build a debug version").conflicts_with("release"),
        ]))
        .subcommand(
            clap::Command::new("qemu")
                .about("Run r9 under QEMU")
                .args(&[
                    clap::arg!(--release "Build a release version").conflicts_with("debug"),
                    clap::arg!(--debug "Build a debug version").conflicts_with("release"),
                ]),
        )
        .subcommand(
            clap::Command::new("qemukvm")
                .about("Run r9 under QEMU with KVM")
                .args(&[
                    clap::arg!(--release "Build a release version").conflicts_with("debug"),
                    clap::arg!(--debug "Build a debug version").conflicts_with("release"),
                ]),
        )
        .subcommand(clap::Command::new("clean").about("Cargo clean"))
        .get_matches();
    if let Err(e) = match matches.subcommand() {
        Some(("build", m)) => build(build_type(m)),
        Some(("expand", m)) => expand(build_type(m)),
        Some(("kasm", m)) => kasm(build_type(m)),
        Some(("dist", m)) => dist(build_type(m)),
        Some(("test", m)) => test(build_type(m)),
        Some(("clippy", m)) => clippy(build_type(m)),
        Some(("qemu", m)) => run(build_type(m)),
        Some(("qemukvm", m)) => accelrun(build_type(m)),
        Some(("clean", _)) => clean(),
        _ => Err("bad subcommand".into()),
    } {
        eprintln!("{}", e);
        process::exit(1);
    }
}

fn build_type(matches: &clap::ArgMatches) -> Build {
    if matches.is_present("release") {
        return Build::Release;
    }
    Build::Debug
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
        let pos = toolchain.find('-').map(|p| p + 1).unwrap_or(0);
        let host = toolchain[pos..].to_string();
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
fn qemu_system_x86_64() -> String {
    env_or("QEMU", "qemu-system-x86_64")
}
fn arch() -> String {
    env_or("ARCH", "x86_64")
}
fn target() -> String {
    env_or("TARGET", "x86_64-unknown-none-elf")
}

fn build(profile: Build) -> Result<()> {
    let mut cmd = Command::new(cargo());
    cmd.current_dir(kernelpath());
    cmd.arg("build");
    #[rustfmt::skip]
    cmd.arg("-Z").arg("build-std=core");
    cmd.arg("--target").arg(format!("lib/{}.json", target()));
    profile.add_build_arg(&mut cmd);
    let status = cmd.status()?;
    if !status.success() {
        return Err("build kernel failed".into());
    }
    Ok(())
}

fn expand(profile: Build) -> Result<()> {
    let mut cmd = Command::new(cargo());
    cmd.current_dir(kernelpath());
    cmd.arg("rustc");
    cmd.arg("-Z").arg("build-std=core");
    cmd.arg("--target").arg(format!("lib/{}.json", target()));
    cmd.arg("--").arg("--pretty=expanded");
    profile.add_build_arg(&mut cmd);
    let status = cmd.status()?;
    if !status.success() {
        return Err("build kernel failed".into());
    }
    Ok(())
}

fn kasm(profile: Build) -> Result<()> {
    let mut cmd = Command::new(cargo());
    cmd.current_dir(kernelpath());
    cmd.arg("build");
    cmd.arg("-Z").arg("build-std=core");
    cmd.arg("--target").arg(format!("lib/{}.json", target()));
    cmd.arg("--").arg("--emit").arg("asm");
    profile.add_build_arg(&mut cmd);
    let status = cmd.status()?;
    if !status.success() {
        return Err("build kernel failed".into());
    }
    Ok(())
}

fn dist(profile: Build) -> Result<()> {
    build(profile)?;
    let mut cmd = Command::new(objcopy());
    cmd.arg("--input-target=elf64-x86-64");
    cmd.arg("--output-target=elf32-i386");
    cmd.arg(format!("target/{}/{}/x86_64", target(), profile.dir()));
    cmd.arg(format!("target/{}/{}/r9.elf32", target(), profile.dir()));
    cmd.current_dir(workspace());
    let status = cmd.status()?;
    if !status.success() {
        return Err("objcopy failed".into());
    }
    Ok(())
}

fn test(profile: Build) -> Result<()> {
    let mut cmd = Command::new(cargo());
    cmd.current_dir(workspace());
    cmd.arg("test");
    profile.add_build_arg(&mut cmd);
    let status = cmd.status()?;
    if !status.success() {
        return Err("test failed".into());
    }
    Ok(())
}

fn clippy(profile: Build) -> Result<()> {
    let mut cmd = Command::new(cargo());
    cmd.current_dir(kernelpath());
    cmd.arg("clippy");
    #[rustfmt::skip]
    cmd.arg("-Z").arg("build-std=core");
    cmd.arg("--target").arg(format!("lib/{}.json", target()));
    profile.add_build_arg(&mut cmd);
    let status = cmd.status()?;
    if !status.success() {
        return Err("build kernel failed".into());
    }
    Ok(())
}

fn run(profile: Build) -> Result<()> {
    dist(profile)?;
    let status = Command::new(qemu_system_x86_64())
        .arg("-nographic")
        //.arg("-curses")
        .arg("-M")
        .arg("q35")
        .arg("-cpu")
        .arg("qemu64,pdpe1gb,xsaveopt,fsgsbase,apic,msr")
        .arg("-smp")
        .arg("8")
        .arg("-m")
        .arg("8192")
        //.arg("-device")
        //.arg("ahci,id=ahci0")
        //.arg("-drive")
        //.arg("id=sdahci0,file=sdahci0.img,if=none")
        //.arg("-device")
        //.arg("ide-hd,drive=sdahci0,bus=ahci0.0")
        .arg("-kernel")
        .arg(format!("target/{}/{}/r9.elf32", target(), profile.dir()))
        .current_dir(workspace())
        .status()?;
    if !status.success() {
        return Err("qemu failed".into());
    }
    Ok(())
}

fn accelrun(profile: Build) -> Result<()> {
    dist(profile)?;
    let status = Command::new(qemu_system_x86_64())
        .arg("-nographic")
        .arg("-accel")
        .arg("kvm")
        .arg("-cpu")
        .arg("host,pdpe1gb,xsaveopt,fsgsbase,apic,msr")
        .arg("-smp")
        .arg("8")
        .arg("-m")
        .arg("8192")
        .arg("-kernel")
        .arg(format!("target/{}/{}/r9.elf32", target(), profile.dir()))
        .current_dir(workspace())
        .status()?;
    if !status.success() {
        return Err("qemu failed".into());
    }
    Ok(())
}

fn clean() -> Result<()> {
    let status = Command::new(cargo())
        .current_dir(workspace())
        .arg("clean")
        .status()?;
    if !status.success() {
        return Err("clean failed".into());
    }
    Ok(())
}

fn workspace() -> PathBuf {
    Path::new(&env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(1)
        .unwrap()
        .to_path_buf()
}

// Returns the path to the kernel package
fn kernelpath() -> PathBuf {
    let mut path = workspace();
    path.push(arch());
    path
}
