use std::{env, ffi, process};

use anyhow::{anyhow, Context};

use crate::{gnu_target, pkg::get_sysroot, status_error, target, toolchain};

pub fn command<S: AsRef<ffi::OsStr>>(program: S) -> anyhow::Result<process::Command> {
    let toolchain_dir = toolchain().context("unable to init toolchain")?;

    // PATH must be set first so cargo is sourced from the toolchain path
    {
        let path = env::var_os("PATH").unwrap_or(ffi::OsString::new());
        let mut paths = env::split_paths(&path).collect::<Vec<_>>();
        paths.insert(0, toolchain_dir.join("bin"));
        let new_path = env::join_paths(paths)?;
        env::set_var("PATH", new_path);
    }

    let ar = format!("{}-ar", gnu_target());
    let cc = format!("{}-gcc", gnu_target());
    let cxx = format!("{}-g++", gnu_target());
    let cc_target_var = target().replace("-", "_");
    let cargo_target_var = cc_target_var.to_uppercase();
    let is_cc = program.as_ref() != "env" && program.as_ref() != "cargo";

    let mut command = process::Command::new(program);
    command.env(format!("AR_{}", cc_target_var), &ar);
    command.env(format!("CARGO_TARGET_{}_LINKER", cargo_target_var), &cc);
    command.env(format!("CC_{}", cc_target_var), &cc);
    command.env(format!("CXX_{}", cc_target_var), &cxx);
    command.env("RUSTUP_TOOLCHAIN", &toolchain_dir);
    command.env("TARGET", target());
    command.env("GNU_TARGET", gnu_target());
    command.env(
        "CFLAGS_riscv64gc_unknown_redox",
        "-march=rv64gc -mabi=lp64d",
    );

    if let Some(sysroot) = get_sysroot() {
        // pkg-config crate specific
        command.env(
            format!("PKG_CONFIG_PATH_{}", cc_target_var),
            sysroot.join("lib/pkgconfig"),
        );
        command.env(
            format!("PKG_CONFIG_SYSROOT_DIR_{}", cc_target_var),
            &sysroot,
        );
        if is_cc {
            let includedir = sysroot.join("include").canonicalize()?;
            let mut cppflags = format!("-I{}", includedir.display());

            if let Ok(user_cppflags) = env::var("CPPFLAGS") {
                cppflags = format!("{} {}", cppflags, user_cppflags);
            }
            if cc_target_var == "riscv64gc_unknown_redox" {
                // TODO: should be set also without sysroot
                cppflags = format!("{} -march=rv64gc -mabi=lp64d", cppflags);
            }

            let libdir = sysroot.join("lib").canonicalize()?;
            let mut ldflags = format!(
                "-Wl,-rpath-link,{} -L{}",
                libdir.display(),
                libdir.display()
            );

            if let Ok(user_ldflags) = env::var("LDFLAGS") {
                ldflags = format!("{} {}", ldflags, user_ldflags);
            }

            command.env("CPPFLAGS", cppflags);
            command.env("LDFLAGS", ldflags);
        }
    }

    Ok(command)
}

fn inner<I: Iterator<Item = String>>(program: &str, args: I) -> anyhow::Result<()> {
    let program = match program {
        "env" => "env".to_string(),
        "ar" => format!("{}-ar", gnu_target()),
        "cc" => format!("{}-gcc", gnu_target()),
        "cxx" => format!("{}-g++", gnu_target()),
        _ => return Err(anyhow!("Unknown env program {:?}", program)),
    };
    command(program)?
        .args(args)
        .status()
        .and_then(status_error)?;

    Ok(())
}

pub fn main(args: &[String]) {
    match inner(args.get(1).unwrap(), args.iter().cloned().skip(2)) {
        Ok(()) => {
            process::exit(0);
        }
        Err(err) => {
            eprintln!("redoxer env: {:#}", err);
            process::exit(1);
        }
    }
}
