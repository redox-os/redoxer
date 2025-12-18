use std::{fs, io, path::PathBuf, process};

static INIT_ENV: &'static str = include_str!("../res/20_env");
static INIT_REDOXER: &'static str = include_str!("../res/29_redoxer");

pub fn write_redoxerd_config(
    dest_dir: &PathBuf,
    arguments: &Vec<String>,
    root_dir: Option<&str>,
) -> Result<(), io::Error> {
    let mut redoxerd_config = String::new();
    for arg in arguments.iter() {
        // Replace absolute path to folder with /root in command name
        // TODO: make this activated by a flag
        if let Some(ref folder) = root_dir {
            let folder_canonical_path = fs::canonicalize(&folder)?;
            let folder_canonical = folder_canonical_path.to_str().ok_or(io::Error::new(
                io::ErrorKind::Other,
                "folder path is not valid UTF-8",
            ))?;
            if arg.starts_with(&folder_canonical) {
                let arg_replace = arg.replace(folder_canonical, "/root");
                eprintln!(
                    "redoxer: replacing '{}' with '{}' in arguments",
                    arg, arg_replace
                );
                redoxerd_config.push_str(&arg_replace);
                redoxerd_config.push('\n');
                continue;
            }
        }

        redoxerd_config.push_str(&arg);
        redoxerd_config.push('\n');
    }
    let etc_dir = dest_dir.join("etc");
    if !etc_dir.is_dir() {
        fs::create_dir_all(&etc_dir)?;
    }
    fs::write(etc_dir.join("redoxerd"), redoxerd_config)?;
    let init_dir = dest_dir.join("usr/lib/init.d");
    if !init_dir.is_dir() {
        fs::create_dir_all(&init_dir)?;
    }
    let init_env_path = init_dir.join("20_env");
    if !init_env_path.is_file() {
        fs::write(&init_env_path, INIT_ENV)?;
    }
    let init_redoxer_path = init_dir.join("29_redoxer");
    if !init_redoxer_path.is_file() {
        fs::write(&init_redoxer_path, INIT_REDOXER)?;
    }
    Ok(())
}

#[derive(Clone, Default)]
struct RedoxerConfig {
    // Root path
    root: PathBuf,
    // Relative directory of execution
    folder: Option<String>,
    // Commands to execute
    arguments: Vec<String>,
}

impl RedoxerConfig {
    pub fn new(mut args: impl Iterator<Item = String>) -> Self {
        let mut config = RedoxerConfig {
            root: std::env::current_dir().expect("Unable to get current dir"),
            ..Default::default()
        };

        // Matching flags
        let mut matching = true;
        while let Some(arg) = args.next() {
            match (arg.as_str(), matching) {
                ("--root", true) => match args.next() {
                    Some(root) => {
                        config.root = PathBuf::from(root);
                    }
                    None => panic!("--root requires a path to a directory"),
                },
                ("--folder", true) => match args.next() {
                    Some(folder) => {
                        config.folder = Some(folder);
                    }
                    None => panic!("--folder requires a path to a directory"),
                },
                ("--", true) => matching = false,
                _ => {
                    matching = false;
                    config.arguments.push(arg);
                }
            }
        }

        config
    }
}

pub fn main(args: &[String]) {
    let config = RedoxerConfig::new(args.iter().cloned().skip(2));

    match write_redoxerd_config(
        &config.root,
        &config.arguments,
        config.folder.as_ref().map(|s| s.as_str()),
    ) {
        Ok(_) => {
            process::exit(0);
        }
        Err(err) => {
            eprintln!("redoxer write-exec: {:#}", err);
            process::exit(1);
        }
    }
}
