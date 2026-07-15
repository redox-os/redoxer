use std::fs::File;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{self, Command};
use std::{env, fs, io};

use sha2::{Digest, Sha256};

use crate::{host_target, redoxer_dir, status_error, target};

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

fn read_shasum<P: AsRef<Path>>(path: P) -> io::Result<Vec<(String, PathBuf)>> {
    let path = path.as_ref();
    let Some(parent) = path.parent() else {
        return Err(io::Error::other("shasum path had no parent"));
    };

    let file = File::open(path)?;
    let reader = BufReader::new(file);

    let mut result = Vec::new();
    for line in reader.lines() {
        let line = line?;
        let line = line.trim();

        if line.len() <= 64 {
            continue;
        }

        // "<hash>  <filename>" or "<hash> *<filename>"
        let (expected_hash, remainder) = line.split_at(64);
        let filename = remainder.trim_start_matches([' ', '*']);
        let target_path = parent.join(filename);
        result.push((expected_hash.to_string(), target_path));
    }

    Ok(result)
}

fn check_shasum(shasum: &Vec<(String, PathBuf)>) -> io::Result<bool> {
    let mut all_match = true;
    let mut checked_any = false;

    for (expected_hash, target_path) in shasum {
        let mut target_file = match File::open(target_path) {
            Ok(f) => f,
            Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
            Err(e) => return Err(e),
        };

        checked_any = true;

        let mut hasher = Sha256::new();
        let mut buffer = [0; 8192];
        loop {
            let count = target_file.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            hasher.update(&buffer[..count]);
        }

        let result = hasher.finalize();
        let actual_hash = format!("{result:x}");

        if actual_hash != *expected_hash {
            all_match = false;
            eprintln!(
                "sha256sum not match for {:?}:",
                target_path.file_name().unwrap_or_default().display(),
            );
            eprintln!("   actual: {actual_hash}");
            eprintln!(" expected: {expected_hash}");
            break;
        }
    }

    Ok(all_match && checked_any)
}

//TODO: Rewrite with hyper or reqwest, tar-rs, sha2, and some gzip crate?
fn toolchain_inner(is_update: bool, source_url: String) -> io::Result<PathBuf> {
    if let Ok(redoxer_toolchain) = env::var("REDOXER_TOOLCHAIN") {
        return Ok(PathBuf::from(redoxer_toolchain));
    }

    let source_is_remote = source_url.starts_with("http://") || source_url.starts_with("https://");
    let url = match source_is_remote {
        true => format!("{}/toolchain/{}/{}", source_url, host_target(), target()),
        false => format!("{}/prefix/{}", source_url, target()),
    };
    let toolchain_dir = redoxer_dir().join("toolchain");
    if is_update && toolchain_dir.is_dir() {
        println!("redoxer: removing old toolchain");

        fs::remove_dir_all(&toolchain_dir)?;
    }
    if !toolchain_dir.is_dir() {
        let toolchain_partial = redoxer_dir().join("toolchain.partial");
        if toolchain_partial.is_dir() {
            fs::remove_dir_all(&toolchain_partial)?;
        }
        fs::create_dir_all(&toolchain_partial)?;

        if source_is_remote {
            const SHASUM_FILENAME: &str = "SHA256SUM";
            const RELIBC_FILENAME: &str = "relibc-install.tar.gz";
            let prefix_tar = toolchain_partial.join(RELIBC_FILENAME);
            println!("redoxer: downloading toolchain from {url:?}");
            let shasum_file = toolchain_partial.join(SHASUM_FILENAME);
            download(&format!("{url}/{SHASUM_FILENAME}"), &shasum_file)?;
            let shasum_data = read_shasum(&shasum_file)?;
            let Some((shasum_hash, _)) = shasum_data.iter().find(|(_, path)| {
                matches!(
                    path.file_name().and_then(|f| f.to_str()),
                    Some(RELIBC_FILENAME)
                )
            }) else {
                println!("redoxer: {shasum_file:?} has no entry for {RELIBC_FILENAME}");
                return Err(io::Error::other("shasum not found"));
            };

            download(
                &format!("{url}/{RELIBC_FILENAME}?cache-buster={shasum_hash}"),
                &prefix_tar,
            )?;

            if !check_shasum(&shasum_data)? {
                return Err(io::Error::other("shasum invalid"));
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
            let prefix_dir = PathBuf::from(format!("{url}/sysroot"));
            if prefix_dir.is_dir() {
                println!("redoxer: copying toolchain from {prefix_dir:?}");
                Command::new("rsync")
                    .arg("-a")
                    .arg(format!("{}/", prefix_dir.display()))
                    .arg(&toolchain_partial)
                    .status()
                    .and_then(status_error)?;
            } else {
                let prefix_tar = format!("{url}/relibc-install.tar.gz");
                println!("redoxer: extracting toolchain from {prefix_tar:?}");
                Command::new("tar")
                    .arg("--extract")
                    .arg("--file")
                    .arg(&prefix_tar)
                    .arg("-C")
                    .arg(&toolchain_partial)
                    .status()
                    .and_then(status_error)?;
            }
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
    let args: Vec<String> = args.iter().skip(2).cloned().collect();

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
                    source_url.clone_from(&args[i + 1]);
                    i += 1;
                } else {
                    eprintln!("Error: --url requires a value.");
                    process::exit(1);
                }
            }
            arg => {
                eprintln!("Warning: Unrecognized argument: {arg}");
            }
        }
        i += 1;
    }

    match toolchain_inner(is_update, source_url) {
        Ok(_) => {
            process::exit(0);
        }
        Err(err) => {
            eprintln!("redoxer toolchain: {err}");
            process::exit(1);
        }
    }
}
