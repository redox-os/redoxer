use std::{env, io, process};
use std::ffi::OsString;
use std::process::Command;

use crate::{status_error, toolchain};

fn inner() -> io::Result<()> {
    let toolchain_dir = toolchain()?;

    let target = "x86_64-unknown-redox";

    let linker = format!("{}-gcc", target);

    let path = {
        let path = env::var_os("PATH").unwrap_or(OsString::new());
        let mut paths = env::split_paths(&path).collect::<Vec<_>>();
        paths.push(toolchain_dir.join("bin"));
        env::join_paths(paths).map_err(|err| io::Error::new(
            io::ErrorKind::Other,
            err
        ))?
    };

    // TODO: Ensure no spaces in toolchain_dir
    let rustflags = format!(
        "-L {}",
        toolchain_dir.join(&target).join("lib").display()
    );

    let mut args = env::args();
    let command = args.next().unwrap();
    let subcommand = args.next().unwrap();

    // TODO: Ensure no spaces in command
    let runner = format!("{} exec --folder .", command);

    Command::new("cargo")
        .arg(subcommand)
        .arg("--target").arg(target)
        .args(args)
        .env("CARGO_TARGET_X86_64_UNKNOWN_REDOX_LINKER", linker)
        .env("CARGO_TARGET_X86_64_UNKNOWN_REDOX_RUNNER", runner)
        .env("PATH", path)
        .env("RUSTFLAGS", rustflags)
        .env("RUSTUP_TOOLCHAIN", &toolchain_dir)
        .env("TARGET", &target)
        .status()
        .and_then(status_error)?;

    Ok(())
}

pub fn main() {
    match inner() {
        Ok(()) => {
            process::exit(0);
        },
        Err(err) => {
            eprintln!("redoxer install: {}", err);
            process::exit(1);
        }
    }
}
