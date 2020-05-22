use std::{env, fs, io};
use std::path::{Path, PathBuf};
use std::process::{self, Command};

use crate::{redoxer_dir, status_error, TARGET};

//TODO: Rewrite with hyper or reqwest, tar-rs, sha2, and some gzip crate?
fn download<P: AsRef<Path>>(url: &str, path: P) -> io::Result<()> {
    Command::new("curl")
        .arg("--proto").arg("=https")
        .arg("--tlsv1.2")
        .arg("--fail")
        .arg("--output").arg(path.as_ref())
        .arg(url)
        .status()
        .and_then(status_error)
}

//TODO: Rewrite with hyper or reqwest, tar-rs, sha2, and some gzip crate?
fn shasum<P: AsRef<Path>>(path: P) -> io::Result<bool> {
    let parent = path.as_ref().parent().ok_or(
        io::Error::new(
            io::ErrorKind::Other,
            "shasum path had no parent"
        )
    )?;
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
pub fn toolchain() -> io::Result<PathBuf> {
    if let Ok(redoxer_toolchain) = env::var("REDOXER_TOOLCHAIN") {
        return Ok(PathBuf::from(redoxer_toolchain));
    }

    let url = format!("https://static.redox-os.org/toolchain/{}", TARGET);
    let toolchain_dir = redoxer_dir().join("toolchain");
    if ! toolchain_dir.is_dir() {
        println!("redoxer: building toolchain");

        let toolchain_partial = redoxer_dir().join("toolchain.partial");
        if toolchain_partial.is_dir() {
            fs::remove_dir_all(&toolchain_partial)?;
        }
        fs::create_dir_all(&toolchain_partial)?;

        let shasum_file = toolchain_partial.join("SHA256SUM");
        download(&format!("{}/SHA256SUM", url), &shasum_file)?;

        let prefix_tar = toolchain_partial.join("rust-install.tar.gz");
        download(&format!("{}/rust-install.tar.gz", url), &prefix_tar)?;

        if ! shasum(&shasum_file)? {
            return Err(io::Error::new(
                io::ErrorKind::Other,
                "shasum invalid"
            ));
        }

        Command::new("tar")
            .arg("--extract")
            .arg("--file").arg(&prefix_tar)
            .arg("-C").arg(&toolchain_partial)
            .arg(".")
            .status()
            .and_then(status_error)?;

        fs::rename(&toolchain_partial, &toolchain_dir)?;
    }

    Ok(toolchain_dir)
}

pub fn main() {
    match toolchain() {
        Ok(_) => {
            process::exit(0);
        },
        Err(err) => {
            eprintln!("redoxer toolchain: {}", err);
            process::exit(1);
        }
    }
}
