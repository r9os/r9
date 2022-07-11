use std::{
    env,
    path::{Path, PathBuf},
    process::{self, Command},
};

type DynError = Box<dyn std::error::Error>;
type Result<T> = std::result::Result<T, DynError>;

#[derive(Clone, Copy)]
enum Profile {
    Debug,
    Release,
}

struct BuildParams {
    profile: Profile,
    verbose: bool,
}

impl BuildParams {
    fn new(matches: &clap::ArgMatches) -> Self {
        let profile =
            if matches.contains_id("release") { Profile::Release } else { Profile::Debug };
        let verbose = matches.contains_id("verbose");
        Self { profile: profile, verbose: verbose }
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
}

fn main() {
    let matches = clap::Command::new("xtask")
        .version("0.1.0")
        .author("The r9 Authors")
        .about("Build support for the r9 operating system")
        .subcommand(clap::Command::new("build").about("Builds r9").args(&[
            clap::arg!(--release "Build release version").conflicts_with("debug"),
            clap::arg!(--debug "Build debug version (default)").conflicts_with("release"),
            clap::arg!(--verbose "Print commands"),
        ]))
        .subcommand(clap::Command::new("expand").about("Expands r9 macros").args(&[
            clap::arg!(--release "Build release version").conflicts_with("debug"),
            clap::arg!(--debug "Build debug version (default)").conflicts_with("release"),
            clap::arg!(--verbose "Print commands"),
        ]))
        .subcommand(clap::Command::new("kasm").about("Emits r9 assembler").args(&[
            clap::arg!(--release "Build release version").conflicts_with("debug"),
            clap::arg!(--debug "Build debug version (default)").conflicts_with("release"),
            clap::arg!(--verbose "Print commands"),
        ]))
        .subcommand(clap::Command::new("dist").about("Builds a multibootable r9 image").args(&[
            clap::arg!(--release "Build a release version").conflicts_with("debug"),
            clap::arg!(--debug "Build a debug version").conflicts_with("release"),
            clap::arg!(--verbose "Print commands"),
        ]))
        .subcommand(clap::Command::new("test").about("Runs unit tests").args(&[
            clap::arg!(--release "Build a release version").conflicts_with("debug"),
            clap::arg!(--debug "Build a debug version").conflicts_with("release"),
            clap::arg!(--verbose "Print commands"),
        ]))
        .subcommand(clap::Command::new("clippy").about("Runs clippy").args(&[
            clap::arg!(--release "Build a release version").conflicts_with("debug"),
            clap::arg!(--debug "Build a debug version").conflicts_with("release"),
            clap::arg!(--verbose "Print commands"),
        ]))
        .subcommand(clap::Command::new("qemu").about("Run r9 under QEMU").args(&[
            clap::arg!(--release "Build a release version").conflicts_with("debug"),
            clap::arg!(--debug "Build a debug version").conflicts_with("release"),
            clap::arg!(--verbose "Print commands"),
        ]))
        .subcommand(clap::Command::new("qemukvm").about("Run r9 under QEMU with KVM").args(&[
            clap::arg!(--release "Build a release version").conflicts_with("debug"),
            clap::arg!(--debug "Build a debug version").conflicts_with("release"),
            clap::arg!(--verbose "Print commands"),
        ]))
        .subcommand(clap::Command::new("clean").about("Cargo clean"))
        .get_matches();

    if let Err(e) = match matches.subcommand() {
        Some(("build", m)) => build(&BuildParams::new(&m)),
        Some(("expand", m)) => expand(&BuildParams::new(&m)),
        Some(("kasm", m)) => kasm(&BuildParams::new(&m)),
        Some(("dist", m)) => dist(&BuildParams::new(&m)),
        Some(("test", m)) => test(&BuildParams::new(&m)),
        Some(("clippy", m)) => clippy(&BuildParams::new(&m)),
        Some(("qemu", m)) => run(&BuildParams::new(&m)),
        Some(("qemukvm", m)) => accelrun(&BuildParams::new(&m)),
        Some(("clean", _)) => clean(),
        _ => Err("bad subcommand".into()),
    } {
        eprintln!("{}", e);
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
fn qemu_system() -> String {
    let defaultqemu = match arch().as_str() {
        "aarch64" => "qemu-system-aarch64",
        _ => "qemu-system-x86_64",
    };
    env_or("QEMU", defaultqemu)
}
fn arch() -> String {
    env_or("ARCH", "x86_64")
}
fn target() -> String {
    env_or("TARGET", "x86_64-unknown-none-elf")
}

fn build(build_params: &BuildParams) -> Result<()> {
    let mut cmd = Command::new(cargo());
    cmd.current_dir(workspace());
    cmd.arg("build");
    #[rustfmt::skip]
    cmd.arg("-Z").arg("build-std=core,alloc");
    cmd.arg("--target").arg(format!("lib/{}.json", target()));
    cmd.arg("--workspace");
    cmd.arg("--exclude").arg("xtask");
    exclude_other_arches(&mut cmd);
    build_params.add_build_arg(&mut cmd);
    if build_params.verbose {
        println!("Executing {:?}", cmd);
    }
    let status = cmd.status()?;
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
    cmd.arg("-p").arg(arch());
    cmd.arg("--target").arg(format!("lib/{}.json", target()));
    cmd.arg("--");
    cmd.arg("-Z").arg("unpretty=expanded");
    build_params.add_build_arg(&mut cmd);
    if build_params.verbose {
        println!("Executing {:?}", cmd);
    }
    let status = cmd.status()?;
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
    cmd.arg("-p").arg(arch());
    cmd.arg("--target").arg(format!("lib/{}.json", target()));
    cmd.arg("--").arg("--emit").arg("asm");
    build_params.add_build_arg(&mut cmd);
    if build_params.verbose {
        println!("Executing {:?}", cmd);
    }
    let status = cmd.status()?;
    if !status.success() {
        return Err("build kernel failed".into());
    }
    Ok(())
}

fn dist(build_params: &BuildParams) -> Result<()> {
    build(build_params)?;

    if arch() == "x86_64" {
        let mut cmd = Command::new(objcopy());
        cmd.arg("--input-target=elf64-x86-64");
        cmd.arg("--output-target=elf32-i386");
        cmd.arg(format!("target/{}/{}/x86_64", target(), build_params.dir()));
        cmd.arg(format!("target/{}/{}/r9.elf32", target(), build_params.dir()));
        cmd.current_dir(workspace());
        if build_params.verbose {
            println!("Executing {:?}", cmd);
        }
        let status = cmd.status()?;
        if !status.success() {
            return Err("objcopy failed".into());
        }
    }
    Ok(())
}

fn test(build_params: &BuildParams) -> Result<()> {
    let mut cmd = Command::new(cargo());
    cmd.current_dir(workspace());
    cmd.arg("test");
    cmd.arg("--workspace");
    exclude_other_arches(&mut cmd);
    build_params.add_build_arg(&mut cmd);
    if build_params.verbose {
        println!("Executing {:?}", cmd);
    }
    let status = cmd.status()?;
    if !status.success() {
        return Err("test failed".into());
    }
    Ok(())
}

fn clippy(build_params: &BuildParams) -> Result<()> {
    let mut cmd = Command::new(cargo());
    cmd.current_dir(workspace());
    cmd.arg("clippy");
    build_params.add_build_arg(&mut cmd);
    if build_params.verbose {
        println!("Executing {:?}", cmd);
    }
    let status = cmd.status()?;
    if !status.success() {
        return Err("build kernel failed".into());
    }
    Ok(())
}

fn run(build_params: &BuildParams) -> Result<()> {
    dist(build_params)?;

    match arch().as_str() {
        "x86_64" => {
            let mut cmd = Command::new(qemu_system());
            cmd.arg("-nographic");
            //cmd.arg("-curses");
            cmd.arg("-M");
            cmd.arg("q35");
            cmd.arg("-cpu");
            cmd.arg("qemu64,pdpe1gb,xsaveopt,fsgsbase,apic,msr");
            cmd.arg("-smp");
            cmd.arg("8");
            cmd.arg("-m");
            cmd.arg("8192");
            //cmd.arg("-device");
            //cmd.arg("ahci,id=ahci0");
            //cmd.arg("-drive");
            //cmd.arg("id=sdahci0,file=sdahci0.img,if=none");
            //cmd.arg("-device");
            //cmd.arg("ide-hd,drive=sdahci0,bus=ahci0.0");
            cmd.arg("-kernel");
            cmd.arg(format!("target/{}/{}/r9.elf32", target(), build_params.dir()));
            cmd.current_dir(workspace());
            if build_params.verbose {
                println!("Executing {:?}", cmd);
            }
            let status = cmd.status()?;
            if !status.success() {
                return Err("qemu failed".into());
            }
        }
        "aarch64" => {
            let mut cmd = Command::new(qemu_system());
            cmd.arg("-nographic");
            //cmd.arg("-curses");
            cmd.arg("-M");
            cmd.arg("raspi3b");
            cmd.arg("-kernel");
            cmd.arg(format!("target/{}/{}/aarch64", target(), build_params.dir()));
            cmd.current_dir(workspace());
            if build_params.verbose {
                println!("Executing {:?}", cmd);
            }
            let status = cmd.status()?;
            if !status.success() {
                return Err("qemu failed".into());
            }
        }
        _ => {
            return Err("Unsupported architecture".into());
        }
    };

    Ok(())
}

fn accelrun(build_params: &BuildParams) -> Result<()> {
    dist(build_params)?;
    let mut cmd = Command::new(qemu_system());
    cmd.arg("-nographic");
    cmd.arg("-accel");
    cmd.arg("kvm");
    cmd.arg("-cpu");
    cmd.arg("host,pdpe1gb,xsaveopt,fsgsbase,apic,msr");
    cmd.arg("-smp");
    cmd.arg("8");
    cmd.arg("-m");
    cmd.arg("8192");
    cmd.arg("-kernel");
    cmd.arg(format!("target/{}/{}/r9.elf32", target(), build_params.dir()));
    cmd.current_dir(workspace());
    if build_params.verbose {
        println!("Executing {:?}", cmd);
    }
    let status = cmd.status()?;
    if !status.success() {
        return Err("qemu failed".into());
    }
    Ok(())
}

fn clean() -> Result<()> {
    let status = Command::new(cargo()).current_dir(workspace()).arg("clean").status()?;
    if !status.success() {
        return Err("clean failed".into());
    }
    Ok(())
}

fn workspace() -> PathBuf {
    Path::new(&env!("CARGO_MANIFEST_DIR")).ancestors().nth(1).unwrap().to_path_buf()
}

// Exclude architectures other than the one being built
fn exclude_other_arches(cmd: &mut Command) {
    match arch().as_str() {
        "x86_64" => {
            cmd.arg("--exclude").arg("aarch64");
        }
        "aarch64" => {
            cmd.arg("--exclude").arg("x86_64");
        }
        _ => {}
    }
}
