use std::{env, ffi, io, process};

use crate::{status_error, toolchain, target};

pub fn command<S: AsRef<ffi::OsStr>>(program: S) -> io::Result<process::Command> {
    let toolchain_dir = toolchain()?;

    // PATH must be set first so cargo is sourced from the toolchain path
    {
        let path = env::var_os("PATH").unwrap_or(ffi::OsString::new());
        let mut paths = env::split_paths(&path).collect::<Vec<_>>();
        paths.insert(0, toolchain_dir.join("bin"));
        let new_path = env::join_paths(paths).map_err(|err| io::Error::new(
            io::ErrorKind::Other,
            err
        ))?;
        env::set_var("PATH", new_path);
    }

    let ar = format!("{}-ar", target());
    let cc = format!("{}-gcc", target());
    let cxx = format!("{}-g++", target());
    let cc_target_var = target().replace("-", "_");
    let cargo_target_var = cc_target_var.to_uppercase();

    let mut command = process::Command::new(program);
    command.env(format!("AR_{}", cc_target_var), &ar);
    command.env(format!("CARGO_TARGET_{}_LINKER", cargo_target_var), &cc);
    command.env(format!("CC_{}", cc_target_var), &cc);
    command.env(format!("CXX_{}", cc_target_var), &cxx);
    command.env("RUSTUP_TOOLCHAIN", &toolchain_dir);
    command.env("TARGET", target());

    Ok(command)
}

fn inner<I: Iterator<Item=String>>(args: I) -> io::Result<()> {
    command("env")?
        .args(args)
        .status()
        .and_then(status_error)?;

    Ok(())
}

pub fn main(args: &[String]) {
    match inner(args.iter().cloned().skip(2)) {
        Ok(()) => {
            process::exit(0);
        },
        Err(err) => {
            eprintln!("redoxer env: {}", err);
            process::exit(1);
        }
    }
}
