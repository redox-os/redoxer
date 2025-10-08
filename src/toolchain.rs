use std::path::{Path, PathBuf};
use std::process::{self, Command};
use std::{env, fs, io};

use crate::{redoxer_dir, status_error, target};

pub const DEFAULT_TOOLCHAIN_SOURCE: &str = "https://static.redox-os.org";

//TODO: Rewrite with hyper or reqwest, tar-rs, sha2, and some gzip crate?
fn download<P: AsRef<Path>>(url: &str, path: P) -> io::Result<()> {
    Command::new("curl")
        .arg("--proto")
        .arg("=https")
        .arg("--tlsv1.2")
        .arg("--fail")
        .arg("--output")
        .arg(path.as_ref())
        .arg(url)
        .status()
        .and_then(status_error)
}

//TODO: Rewrite with hyper or reqwest, tar-rs, sha2, and some gzip crate?
fn shasum<P: AsRef<Path>>(path: P) -> io::Result<bool> {
    let parent = path.as_ref().parent().ok_or(io::Error::new(
        io::ErrorKind::Other,
        "shasum path had no parent",
    ))?;
    Command::new("sha256sum")
        .arg("--check")
        .arg("--ignore-missing")
        .arg("--quiet")
        .arg(path.as_ref())
        .current_dir(parent)
        .status()
        .map(|status| status.success())
}

//TODO: Rewrite with hyper or reqwest, tar-rs, sha2, and some gzip crate?
fn toolchain_inner(is_update: bool, source_url: String) -> io::Result<PathBuf> {
    if let Ok(redoxer_toolchain) = env::var("REDOXER_TOOLCHAIN") {
        return Ok(PathBuf::from(redoxer_toolchain));
    }

    let source_is_remote = source_url.starts_with("http://") || source_url.starts_with("https://");
    let url = match source_is_remote {
        true => format!("{}/toolchain/{}", source_url, target()),
        false => format!("{}/prefix/{}", source_url, target()),
    };
    let toolchain_dir = redoxer_dir().join("toolchain");
    if is_update && toolchain_dir.is_dir() {
        println!("redoxer: removing old toolchain");

        fs::remove_dir_all(&toolchain_dir)?;
    }
    if !toolchain_dir.is_dir() {
        println!("redoxer: building toolchain");

        let toolchain_partial = redoxer_dir().join("toolchain.partial");
        if toolchain_partial.is_dir() {
            fs::remove_dir_all(&toolchain_partial)?;
        }
        fs::create_dir_all(&toolchain_partial)?;

        if source_is_remote {
            let shasum_file = toolchain_partial.join("SHA256SUM");
            download(&format!("{}/SHA256SUM", url), &shasum_file)?;

            let prefix_tar = toolchain_partial.join("rust-install.tar.gz");
            download(&format!("{}/rust-install.tar.gz", url), &prefix_tar)?;

            if !shasum(&shasum_file)? {
                return Err(io::Error::new(io::ErrorKind::Other, "shasum invalid"));
            }

            Command::new("tar")
                .arg("--extract")
                .arg("--file")
                .arg(&prefix_tar)
                .arg("-C")
                .arg(&toolchain_partial)
                .status()
                .and_then(status_error)?;

            fs::remove_file(&shasum_file)?;
            fs::remove_file(&prefix_tar)?;
        } else {
            let prefix_tar = format!("{}/relibc-install.tar.gz", url);

            Command::new("tar")
                .arg("--extract")
                .arg("--file")
                .arg(&prefix_tar)
                .arg("-C")
                .arg(&toolchain_partial)
                .status()
                .and_then(status_error)?;
        }

        fs::rename(&toolchain_partial, &toolchain_dir)?;
    }

    Ok(toolchain_dir)
}

pub fn toolchain() -> io::Result<PathBuf> {
    toolchain_inner(false, String::from(DEFAULT_TOOLCHAIN_SOURCE))
}

pub fn main(args: &[String]) {
    let mut is_update = false;
    let mut source_url: String = String::from(DEFAULT_TOOLCHAIN_SOURCE);
    let args: Vec<String> = args.iter().cloned().skip(2).collect();

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--help" => {
                eprintln!("redoxer toolchain [--update] [--url PATH]");
                eprintln!("  --update     update existing toolchain");
                eprintln!(
                    "  --url PATH   use PATH as source URL instead of {}",
                    DEFAULT_TOOLCHAIN_SOURCE
                );
                eprintln!("               PATH can be a local path (to copy) or http(s) URL (to download)");
                eprintln!("               local PATH is used to update relibc inside redoxer");
                process::exit(0);
            }
            "--update" => {
                is_update = true;
            }
            "--url" => {
                if i + 1 < args.len() {
                    source_url = args[i + 1].clone();
                    i += 1;
                } else {
                    eprintln!("Error: --url requires a value.");
                    process::exit(1);
                }
            }
            arg => {
                eprintln!("Warning: Unrecognized argument: {}", arg);
            }
        }
        i += 1;
    }

    match toolchain_inner(is_update, source_url) {
        Ok(_) => {
            process::exit(0);
        }
        Err(err) => {
            eprintln!("redoxer toolchain: {}", err);
            process::exit(1);
        }
    }
}
