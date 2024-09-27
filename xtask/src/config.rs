/// Test
///
use crate::{Command, Profile};

use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs::{self, create_dir_all, File},
    io::Write,
    process::exit,
};

/// build section
#[derive(Debug, Serialize, Deserialize)]
pub struct Build {
    /// The buildflags controls build-time operations and compiler settings.
    pub buildflags: Option<Vec<String>>,

    /// A list of custom flags to pass to all compiler invocations that Cargo performs.
    pub rustflags: Option<Vec<String>>,

    /// Build for the given architecture.
    pub target: String,
}

/// Config section
/// currently available configuration sections are dev, ip, link, nodev, nouart
/// the section name is becomes the prefix for the configuration option
/// example usage for section "dev"
/// ```toml
///  dev = [
///     'arch',
///     'cap',
///     'foo="baz"'
///  ]
/// ```
///  this will create the following configuration options
///  dev_arch, dev_cap and dev_foo="baz"
///
/// usage example:
///  ```rust
/// #[cfg(dev_arch)]
/// pub mod devarch;
/// ```
/// ```rust
/// #[cfg(dev_foo = "baz")]
/// pub mod foobaz;
/// ```
#[derive(Debug, Serialize, Deserialize)]
pub struct Config {
    pub dev: Option<Vec<String>>,
    pub features: Option<Vec<String>>,
    pub ip: Option<Vec<String>>,
    pub link: Option<Vec<String>>,
    pub nodev: Option<Vec<String>>,
    pub nouart: Option<Vec<String>>,

    /// platform/board possible values: empty, raspi3b, vfive2, nezha, virt etc.
    ///
    /// example usage
    /// ´´´rust
    /// #[cfg(platform = "virt")]
    /// pub mod virt;
    /// ```
    pub platform: Option<String>,

    /// Filepath of DTB file relative to crate
    pub dtb: Option<String>,
}

/// Qemu section
/// Affects arguments to be passed to qemu - doesn't affect build artefacts.
#[derive(Debug, Serialize, Deserialize)]
pub struct Qemu {
    /// Machine (`-M`) value for qemu: raspi3b, raspi4b, etc.
    pub machine: Option<String>,

    /// Filepath of DTB file relative to crate
    pub dtb: Option<String>,
}

/// the TOML document
#[derive(Debug, Serialize, Deserialize)]
pub struct Configuration {
    pub build: Option<Build>,
    pub config: Option<Config>,
    pub link: Option<HashMap<String, String>>,
    pub qemu: Option<Qemu>,
}

impl Configuration {
    pub fn load(filename: String) -> Self {
        let contents = match fs::read_to_string(filename.clone()) {
            Ok(c) => c,
            Err(_) => {
                eprintln!("Could not read file `{filename}`");
                exit(1);
            }
        };
        let config: Configuration = match toml::from_str(&contents) {
            Ok(d) => d,
            Err(e) => {
                eprintln!("TOML: Unable to load data from `{}`", filename);
                eprintln!("{e}");
                exit(1);
            }
        };
        config
    }
}

fn apply_build(cmd: &mut Command, rustflags: &mut Vec<String>, config: &Configuration) {
    if let Some(config) = &config.build {
        let target = &config.target;
        cmd.arg("--target").arg(target);

        if let Some(flags) = &config.buildflags {
            // add the buildflags to the command
            for f in flags {
                cmd.arg(f);
            }
        }

        if let Some(flags) = &config.rustflags {
            // store the passed rustflags temporarily
            for f in flags {
                rustflags.push(f.to_string());
            }
        }
    }
}

fn apply_platform_config(cmd: &mut Command, rustflags: &mut Vec<String>, config: &Configuration) {
    if let Some(config) = &config.config {
        // if the target will use features make them available
        if let Some(features) = &config.features {
            let mut joined = features.join(",");
            if !features.is_empty() && joined.is_empty() {
                joined = features.first().unwrap().into();
            }
            cmd.arg(format!("--features={joined}"));
        }

        if let Some(platform) = &config.platform {
            rustflags.push("--cfg".into());
            rustflags.push(format!("platform=\"{}\"", platform));
        }

        if let Some(devices) = &config.dev {
            // get all [config] 'dev' settings
            for dev in devices {
                rustflags.push("--cfg".into());

                // prefix the setting
                rustflags.push(format!("dev_{dev}"));
            }
        }

        if let Some(ips) = &config.ip {
            // get all [config] 'ip' settings
            for ip in ips {
                rustflags.push("--cfg".into());

                // prefix the setting
                rustflags.push(format!("ip_{ip}"));
            }
        }
        if let Some(links) = &config.link {
            // get all [config] 'link' settings
            for link in links {
                rustflags.push("--cfg".into());

                // prefix the setting
                rustflags.push(format!("link_{link}"));
            }
        }

        if let Some(nodevs) = &config.nodev {
            // get all [config] 'nodev' settings
            for nodev in nodevs {
                rustflags.push("--cfg".into());

                // prefix the setting
                rustflags.push(format!("nodev_{nodev}"));
            }
        }

        if let Some(nouarts) = &config.nouart {
            // get all [config] 'nodev' settings
            for nouart in nouarts {
                rustflags.push("--cfg".into());

                // prefix the setting
                rustflags.push(format!("nouart_{nouart}"));
            }
        }
    }
}

fn apply_link(
    rustflags: &mut Vec<String>,
    config: &Configuration,
    target: &str,
    profile: &Profile,
    workspace_path: &str,
) {
    // we don't need to handle the linker script for clippy
    if let Some(link) = &config.link {
        let filename = link["script"].clone();

        // do we have a linker script ?
        if !filename.is_empty() {
            let mut contents = match fs::read_to_string(format!("{}/{}", workspace_path, filename))
            {
                Ok(c) => c,
                Err(_) => {
                    eprintln!("Could not read file `{filename}`");
                    exit(1);
                }
            };

            // replace the placeholders with the values from the TOML
            if let Some(link) = &config.link {
                for l in link.iter() {
                    match l.0.as_str() {
                        "arch" => contents = contents.replace("${ARCH}", l.1),
                        "load-address" => contents = contents.replace("${LOAD-ADDRESS}", l.1),
                        "script" => {} // do nothing for the script option
                        _ => eprintln!("ignoring unknown option '{} = {}'", l.0, l.1),
                    }
                }
            }

            // construct the path to the target directory
            let path = format!(
                "{}/target/{}/{}",
                workspace_path,
                target,
                profile.to_string().to_lowercase()
            );

            // make sure the target directory exists
            if !std::path::Path::new(&path).exists() {
                // if not, create it
                let _ = create_dir_all(&path);
            }

            // everything is setup, now create the linker script
            // in the target directory
            let mut file = File::create(format!("{}/kernel.ld", path)).unwrap();
            let _ = file.write_all(contents.as_bytes());

            // pass the script path to the rustflags
            rustflags.push(format!("-Clink-args=-T{}/kernel.ld", path));
        }
    }
}

fn apply_qemu_config(cmd: &mut Command, config: &Configuration) {
    if let Some(config) = &config.qemu {
        if let Some(machine) = &config.machine {
            cmd.arg("-M");
            cmd.arg(machine);
        }
        if let Some(dtb) = &config.dtb {
            cmd.arg("-dtb");
            cmd.arg(dtb);
        }
    }
}

fn apply_rustflags(cmd: &mut Command, rustflags: &[String]) {
    // pass the collected rustflags
    // !! this overrides the build.rustflags from the target Cargo.toml !!
    if !rustflags.is_empty() {
        let flat = rustflags.join(" ");
        cmd.arg("--config");
        cmd.arg(format!("build.rustflags='{}'", flat));
    }
}

pub fn apply_to_clippy_step(cmd: &mut Command, config: &Configuration) {
    let mut rustflags: Vec<String> = Vec::new();
    apply_platform_config(cmd, &mut rustflags, config);
    apply_rustflags(cmd, &rustflags);
}

pub fn apply_to_build_step(
    cmd: &mut Command,
    config: &Configuration,
    target: &str,
    profile: &Profile,
    workspace_path: &str,
) {
    let mut rustflags: Vec<String> = Vec::new();
    apply_build(cmd, &mut rustflags, config);
    apply_platform_config(cmd, &mut rustflags, config);
    apply_link(&mut rustflags, config, target, profile, workspace_path);
    apply_rustflags(cmd, &rustflags);
}

pub fn apply_to_qemu_step(cmd: &mut Command, config: &Configuration) {
    apply_qemu_config(cmd, config);
}
