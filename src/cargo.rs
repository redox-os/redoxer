use std::{env, io, process};
use std::ffi::OsString;
use std::process::Command;

use crate::{status_error, toolchain};

fn inner() -> io::Result<()> {
    let toolchain_dir = toolchain()?;

    let target = "x86_64-unknown-redox";

    let linker = format!("{}-gcc", target);

    // PATH must be set first so cargo is sourced from the toolchain path
    {
        let path = env::var_os("PATH").unwrap_or(OsString::new());
        let mut paths = env::split_paths(&path).collect::<Vec<_>>();
        paths.insert(0, toolchain_dir.join("bin"));
        let new_path = env::join_paths(paths).map_err(|err| io::Error::new(
            io::ErrorKind::Other,
            err
        ))?;
        env::set_var("PATH", new_path);
    }

    // TODO: Ensure no spaces in toolchain_dir
    let rustflags = format!(
        "-L {}",
        toolchain_dir.join(&target).join("lib").display()
    );

    let mut args = env::args();
    let command = args.next().unwrap();
    let subcommand = args.next().unwrap();

    let mut arguments = Vec::new();
    let mut matching = true;
    let mut gui = false;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-g" | "--gui" if matching => {
                gui = true;
            },
            "--" if matching => {
                matching = false;
                arguments.push(arg);
            }
            _ => {
                arguments.push(arg);
            }
        }
    }

    // TODO: Ensure no spaces in command
    let runner = format!(
        "{} exec --folder .{}",
        command,
        if gui { " --gui" } else { "" }
    );

    Command::new("cargo")
        .arg(subcommand)
        .arg("--target").arg(target)
        .args(arguments)
        .env("CARGO_TARGET_X86_64_UNKNOWN_REDOX_LINKER", linker)
        .env("CARGO_TARGET_X86_64_UNKNOWN_REDOX_RUNNER", runner)
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
            eprintln!("redoxer cargo: {}", err);
            process::exit(1);
        }
    }
}
