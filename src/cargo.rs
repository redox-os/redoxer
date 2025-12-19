use std::ffi::OsString;
use std::{env, io, process};

use anyhow::Context;

use crate::exec::RedoxerExecConfig;
use crate::{status_error, target, toolchain};

fn inner<I: Iterator<Item = String>>(mut args: I) -> anyhow::Result<()> {
    let toolchain_dir = toolchain()?;

    // PATH must be set first so cargo is sourced from the toolchain path
    {
        let path = env::var_os("PATH").unwrap_or(OsString::new());
        let mut paths = env::split_paths(&path).collect::<Vec<_>>();
        paths.insert(0, toolchain_dir.join("bin"));
        let new_path =
            env::join_paths(paths).map_err(|err| io::Error::new(io::ErrorKind::Other, err))?;
        unsafe {
            env::set_var("PATH", new_path);
        }
    }

    // Use CARGO_ENCODED_RUSTFLAGS to handle spaces
    let mut rustflags = format!(
        "-L\x1fnative={}",
        toolchain_dir.join(target()).join("lib").display()
    );

    if let Ok(user_rustflag) = env::var("RUSTFLAGS") {
        rustflags = format!("{}\x1f{}", rustflags, user_rustflag.replace(" ", "\x1f"));
    }

    #[cfg(feature = "cli-pkg")]
    if let Some(sysroot) = crate::pkg::get_sysroot() {
        rustflags = format!(
            "{}\x1f-L\x1fnative={}",
            rustflags,
            sysroot.join("lib").canonicalize()?.display()
        );
    }

    let command = args.next().unwrap();
    let subcommand = args.next().unwrap();

    let mut runner_config =
        RedoxerExecConfig::new(args).context("Unable to parse exec configuration")?;
    let arguments = runner_config.arguments.clone();
    runner_config.arguments = Vec::new();
    runner_config
        .folders
        .insert("root".to_string(), ".".to_string());

    let mut runner = vec![command, "exec".to_string()];
    runner.extend(runner_config.to_args().into_iter().map(|s| {
        if s.contains(&[' ', '"', '\'', '\n']) {
            format!("{:?}", s)
        } else {
            s
        }
    }));

    let cc_target_var = target().replace("-", "_");
    let cargo_target_var = cc_target_var.to_uppercase();

    crate::env::command("cargo")?
        .arg(subcommand)
        .arg("--target")
        .arg(target())
        .args(arguments)
        .env(
            format!("CARGO_TARGET_{}_RUNNER", cargo_target_var),
            runner.join(" "),
        )
        .env("CARGO_ENCODED_RUSTFLAGS", rustflags)
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
