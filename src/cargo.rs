use std::ffi::OsString;
use std::{env, io, process};

use crate::{status_error, target, toolchain};

fn inner<I: Iterator<Item = String>>(mut args: I) -> io::Result<()> {
    let toolchain_dir = toolchain()?;

    // PATH must be set first so cargo is sourced from the toolchain path
    {
        let path = env::var_os("PATH").unwrap_or(OsString::new());
        let mut paths = env::split_paths(&path).collect::<Vec<_>>();
        paths.insert(0, toolchain_dir.join("bin"));
        let new_path =
            env::join_paths(paths).map_err(|err| io::Error::new(io::ErrorKind::Other, err))?;
        env::set_var("PATH", new_path);
    }

    // TODO: Ensure no spaces in toolchain_dir
    let rustflags = format!(
        "{} -L native={}",
        env::var("RUSTFLAGS").unwrap_or_default(),
        toolchain_dir.join(target()).join("lib").display()
    );

    let command = args.next().unwrap();
    let subcommand = args.next().unwrap();

    let mut arguments = Vec::new();
    let mut matching = true;
    let mut gui = false;
    let mut output_opt = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-g" | "--gui" if matching => {
                gui = true;
            }
            "-o" | "--output" if matching => match args.next() {
                Some(output) => {
                    output_opt = Some(output);
                }
                None => {
                    //TODO: usage();
                }
            },
            "--" if matching => {
                matching = false;
            }
            _ => {
                arguments.push(arg);
            }
        }
    }

    // TODO: Ensure no spaces in command
    let runner = format!(
        "{} exec --folder .{}{}",
        command,
        if gui { " --gui" } else { "" },
        match output_opt {
            Some(output) => format!(" --output {}", output),
            None => String::new(),
        }
    );

    crate::env::command("cargo")?
        .arg(subcommand)
        .arg("--target")
        .arg(target())
        .args(arguments)
        .env("CARGO_TARGET_X86_64_UNKNOWN_REDOX_RUNNER", runner)
        .env("RUSTFLAGS", rustflags)
        .status()
        .and_then(status_error)?;

    Ok(())
}

pub fn main(args: &[String]) {
    match inner(args.iter().cloned()) {
        Ok(()) => {
            process::exit(0);
        }
        Err(err) => {
            eprintln!("redoxer cargo: {}", err);
            process::exit(1);
        }
    }
}
