use crate::{cargo, Command, Profile};

use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs::{self, create_dir_all, File},
    io::Write,
    process::exit,
};

#[derive(Debug, Serialize, Deserialize)]
pub struct Build {
    pub buildflags: Option<Vec<String>>,
    pub rustflags: Option<Vec<String>>,
    pub target: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Link {
    pub conf: HashMap<String, String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Config {
    pub dev: Option<Vec<String>>,
    pub features: Option<Vec<String>>,
    pub ip: Option<Vec<String>>,
    pub link: Option<Vec<String>>,
    pub nodev: Option<Vec<String>>,
    pub nouart: Option<Vec<String>>,
    pub platform: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Configuration {
    pub build: Option<Build>,
    pub config: Option<Config>,
    pub link: Option<HashMap<String, String>>,
}

pub fn read_config(filename: String) -> Configuration {
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
            eprintln!("ConfigFile: Unable to load data from `{}`", filename);
            eprintln!("{e}");
            exit(1);
        }
    };
    config
}

pub fn generate_args(
    task: &str,
    config: &Configuration,
    target: &str,
    profile: &Profile,
    wks_path: &str,
) -> Command {
    let mut rustflags: Vec<String> = Vec::new();
    let mut cmd = Command::new(cargo());
    cmd.arg(task);

    if let Some(config) = &config.build {
        if task != "clippy" {
            let target = &config.target;
            cmd.arg("--target").arg(target);

            if let Some(flags) = &config.buildflags {
                for f in flags {
                    cmd.arg(f);
                }
            }

            if let Some(flags) = &config.rustflags {
                for f in flags {
                    rustflags.push(f.to_string());
                }
            }
        }
    }

    if let Some(config) = &config.config {
        rustflags.push("--cfg".into());
        rustflags.push(format!("platform=\"{}\"", config.platform));

        if let Some(features) = &config.features {
            let mut joined = features.join(",");
            if !features.is_empty() && joined.is_empty() {
                joined = features.first().unwrap().into();
            }
            cmd.arg(format!("--features={joined}"));
        }

        if let Some(devices) = &config.dev {
            for dev in devices {
                rustflags.push("--cfg".into());
                rustflags.push(format!("dev_{dev}"));
            }
        }

        if let Some(ips) = &config.ip {
            for ip in ips {
                rustflags.push("--cfg".into());
                rustflags.push(format!("ip_{ip}"));
            }
        }
        if let Some(links) = &config.link {
            for link in links {
                rustflags.push("--cfg".into());
                rustflags.push(format!("link_{link}"));
            }
        }

        if let Some(nodevs) = &config.nodev {
            for nodev in nodevs {
                rustflags.push("--cfg".into());
                rustflags.push(format!("nodev_{nodev}"));
            }
        }

        if let Some(nouarts) = &config.nouart {
            for nouart in nouarts {
                rustflags.push("--cfg".into());
                rustflags.push(format!("nouart_{nouart}"));
            }
        }
    }

    if task != "clippy" {
        if let Some(link) = &config.link {
            let filename = link["script"].clone();

            if !filename.is_empty() {
                let mut contents = match fs::read_to_string(format!("{}/{}", wks_path, filename)) {
                    Ok(c) => c,
                    Err(_) => {
                        eprintln!("Could not read file `{filename}`");
                        exit(1);
                    }
                };

                if let Some(link) = &config.link {
                    for l in link.iter() {
                        match l.0.as_str() {
                            "arch" => contents = contents.replace("${ARCH}", l.1),
                            "load-address" => contents = contents.replace("${LOAD-ADDRESS}", l.1),
                            "script" => {}
                            _ => eprintln!("ignoring unknown option '{} = {}'", l.0, l.1),
                        }
                    }
                }

                let path = format!(
                    "{}/target/{}/{}",
                    wks_path,
                    target,
                    profile.to_string().to_lowercase()
                );
                if !std::path::Path::new(&path).exists() {
                    let _ = create_dir_all(&path);
                }

                let mut file = File::create(format!("{}/kernel.ld", path)).unwrap();
                let _ = file.write_all(contents.as_bytes());

                rustflags.push(format!("-Clink-args=-T{}/kernel.ld", path));
            }
        }
    }
    if !rustflags.is_empty() {
        let flat = rustflags.join(" ");
        cmd.arg("--config");
        cmd.arg(format!("build.rustflags='{}'", flat));
    }

    cmd
}
