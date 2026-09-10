use std::{collections::HashMap, env, ffi, process};

use anyhow::{anyhow, Context};

use crate::{gnu_target, host_target, status_error, target, toolchain};

fn target_is_64bit(target: &'static str) -> bool {
    !matches!(&target[0..4], "i586" | "i686")
}

fn append_flag(buf: &mut String, flag: &'static str) {
    if !buf.is_empty() {
        buf.push(' ');
    }
    buf.push_str(flag);
}

fn append_flag2(buf: &mut String, flag: &'static str, flag2: &str) {
    if !buf.is_empty() {
        buf.push(' ');
    }
    buf.push_str(flag);
    // TODO: Quote spaces
    buf.push_str(flag2);
}

pub fn command<S: AsRef<ffi::OsStr>>(program: S) -> anyhow::Result<process::Command> {
    let toolchain_dir = toolchain().context("unable to init toolchain")?;

    let mut command = process::Command::new(program.as_ref());
    if std::env::var("REDOXER_REENTRANT").is_ok() {
        // TODO: do not wrap cargo as redoxer_cookbook and erase this logic
        return Ok(command);
    }

    // PATH must be set first so cargo is sourced from the toolchain path
    {
        let path = env::var_os("PATH").unwrap_or_default();
        let mut paths = env::split_paths(&path).collect::<Vec<_>>();
        paths.insert(0, toolchain_dir.join("bin"));
        let new_path = env::join_paths(paths)?;
        unsafe {
            env::set_var("PATH", new_path);
        }
    }

    // CC
    let target = target();
    let gnu_target = gnu_target();
    let gnu_targets = generate_gnu_targets();
    let cc_target_var = target.replace("-", "_");
    let cargo_target_var = cc_target_var.to_uppercase();
    let is_clang = crate::is_use_clang();
    let is_lto = crate::is_use_lto();
    let is_cc = program.as_ref() != "env" && program.as_ref() != "cargo";
    let is_host = host_target() == target;
    for (k, v) in gnu_targets.iter() {
        if (*k == "CC" || *k == "CXX")
            && let Ok(cc_wrapper) = std::env::var("CC_WRAPPER")
            && !cc_wrapper.is_empty()
        {
            command.env(k, format!("{cc_wrapper} {v}"));
            continue;
        }
        command.env(k, v);
        command.env(format!("{k}_{cc_target_var}"), v);
    }

    // CARGO
    command.env(
        format!("CARGO_TARGET_{cargo_target_var}_LINKER"),
        &if is_clang {
            format!("{target}-clang")
        } else {
            gnu_targets.get("CC").unwrap().to_string()
        },
    );
    command.env("RUSTUP_TOOLCHAIN", &toolchain_dir);
    command.env("TARGET", target);
    command.env("GNU_TARGET", gnu_target);

    // RUSTFLAGS, TODO:
    // 1. we're setting global RUSTFLAGS to per-target RUSTFLAGS
    //      without a way to let user leave global RUSTFLAGS
    //      but that probably is ok, because no usecase to it
    // 2. Global RUSTFLAGS is really confusing because of this issue
    //      https://github.com/rust-lang/cargo/issues/4423
    //      which claims there's no RUSTFLAGS for build.rs
    // 3. There are no CARGO_TARGET_xxx_ENCODED_RUSTFLAGS
    let rustflags_env = if is_host {
        "REDOXER_HOST_RUSTFLAGS"
    } else {
        "RUSTFLAGS"
    };
    let mut rustflags = env::var(rustflags_env).unwrap_or_default();
    if target_is_64bit(target) {
        append_flag(&mut rustflags, "-C force-frame-pointers=yes");
    }
    if is_clang && is_lto {
        // only with clang that LTO can work in rust
        append_flag(&mut rustflags, "-C lto=thin -C linker-plugin-lto");
    }

    // CPPFLAGS
    let cppflags_env = if is_host {
        "REDOXER_HOST_CPPFLAGS"
    } else {
        "CPPFLAGS"
    };
    let mut cppflags = env::var(cppflags_env).unwrap_or_else(|_| String::new());
    match target {
        "aarch64-unknown-redox" => append_flag(&mut cppflags, "-mno-outline-atomics"),
        "riscv64gc-unknown-redox" => append_flag(&mut cppflags, "-march=rv64gc -mabi=lp64d"),
        _ => {}
    }
    if is_lto {
        append_flag(
            &mut cppflags,
            if is_clang {
                "-flto=thin"
            } else {
                "-flto=auto -fno-fat-lto-objects"
            },
        );
    }

    // LDFLAGS
    let ldflags_env = if is_host {
        "REDOXER_HOST_LDFLAGS"
    } else {
        "LDFLAGS"
    };
    #[allow(unused_mut)]
    let mut ldflags = env::var(ldflags_env).unwrap_or_default();

    if is_lto {
        // all CPPFLAGS need to be passed again to LDFLAGS (unquoted)
        append_flag2(&mut ldflags, "", &cppflags);
    }

    if is_host && is_clang {
        if cfg!(target_os = "linux") {
            // LLVMgold.so is not shipped to the toolchain
            append_flag(&mut ldflags, "-fuse-ld=lld");
        }
    }

    #[cfg(feature = "cli-pkg")]
    if let Some(sysroot) = crate::pkg::get_sysroot() {
        // pkg-config crate specific
        command.env(
            format!("PKG_CONFIG_PATH_{cc_target_var}"),
            sysroot.join("lib/pkgconfig"),
        );
        command.env(format!("PKG_CONFIG_SYSROOT_DIR_{cc_target_var}"), &sysroot);
        // we've set `PKG_CONFIG_PATH` and prefixed pkg-config isn't available
        command.env("PKG_CONFIG", "pkg-config");
        command.env(format!("PKG_CONFIG_{cc_target_var}"), "pkg-config");

        let includedir = sysroot.join("include").canonicalize()?;
        if let Some(includedir) = includedir.to_str() {
            append_flag2(&mut cppflags, "-I", includedir);
        }
        let libdir = sysroot.join("lib").canonicalize()?;
        if let Some(libdir) = libdir.to_str() {
            append_flag(&mut rustflags, "-C target-feature=-crt-static");
            append_flag2(&mut rustflags, "-L native=", libdir);
            append_flag2(&mut rustflags, "-C link-arg=-Wl,-rpath-link,", libdir);
            append_flag2(&mut ldflags, "-Wl,-rpath-link,", libdir);
            append_flag2(&mut ldflags, "-L", libdir);
        }
    }

    if !cppflags.is_empty() {
        command.env("CPPFLAGS", &cppflags);
        command.env(format!("CFLAGS_{cc_target_var}"), &cppflags);
        command.env(format!("CXXFLAGS_{cc_target_var}"), &cppflags);
        if is_cc {
            command.args(cppflags.split_ascii_whitespace());
        }
    } else if is_host {
        command.env_remove("CPPFLAGS");
        command.env_remove(format!("CFLAGS_{cc_target_var}"));
        command.env_remove(format!("CXXFLAGS_{cc_target_var}"));
    }
    if !ldflags.is_empty() {
        command.env("LDFLAGS", &ldflags);
        if is_cc {
            command.args(ldflags.split_ascii_whitespace());
        }
    } else if is_host {
        command.env_remove("LDFLAGS");
    }
    if !rustflags.is_empty() {
        command.env(
            format!("CARGO_TARGET_{cargo_target_var}_RUSTFLAGS"),
            rustflags,
        );
        command.env_remove("RUSTFLAGS");
    } else if is_host {
        command.env_remove("RUSTFLAGS");
    }

    command.env("REDOXER_REENTRANT", "1");

    Ok(command)
}

fn inner<I: Iterator<Item = String>>(program: &str, args: I) -> anyhow::Result<()> {
    let clang = crate::is_use_clang();
    let program = match program {
        "env" => "env".to_string(),
        "ar" if clang => "llvm-ar".to_string(),
        "cc" if clang => "clang".to_string(),
        "cxx" if clang => "clang++".to_string(),
        "ar" => format!("{}-ar", gnu_target()),
        "cc" => format!("{}-gcc", gnu_target()),
        "cxx" => format!("{}-g++", gnu_target()),
        _ => return Err(anyhow!("Unknown env program {program:?}")),
    };
    command(program)?
        .args(args)
        .status()
        .and_then(status_error)?;

    Ok(())
}

fn generate_gnu_targets() -> HashMap<&'static str, String> {
    let is_host = host_target() == target();
    let mut h = HashMap::new();
    let target_prefix = if is_host {
        String::new()
    } else {
        format!("{}-", gnu_target())
    };
    if !crate::is_use_clang() {
        h.insert("AR", format!("{target_prefix}gcc-ar"));
        h.insert("AS", format!("{target_prefix}as"));
        h.insert("CC", format!("{target_prefix}gcc"));
        h.insert("CXX", format!("{target_prefix}g++"));
        h.insert("LD", format!("{target_prefix}ld"));
        h.insert("NM", format!("{target_prefix}gcc-nm"));
        h.insert("OBJCOPY", format!("{target_prefix}objcopy"));
        h.insert("OBJDUMP", format!("{target_prefix}objdump"));
        h.insert("PKG_CONFIG", format!("{target_prefix}pkg-config"));
        h.insert("RANLIB", format!("{target_prefix}gcc-ranlib"));
        h.insert("READELF", format!("{target_prefix}readelf"));
        h.insert("STRIP", format!("{target_prefix}strip"));
    } else {
        let target_flag = if is_host {
            String::new()
        } else {
            format!(" --target={}", gnu_target())
        };

        let target_cxxflag = if is_host {
            String::new()
        } else {
            // TODO: libcxx is not ready (see llvm-rt21 recipe)
            " -stdlib=libstdc++".to_string()
        };

        h.insert("AR", "llvm-ar".to_string());
        h.insert("LD", "ld.lld".to_string());
        h.insert("NM", "llvm-nm".to_string());
        h.insert("OBJCOPY", "llvm-objcopy".to_string());
        h.insert("OBJDUMP", "llvm-objdump".to_string());
        h.insert("RANLIB", "llvm-ranlib".to_string());
        h.insert("READELF", "llvm-readelf".to_string());
        h.insert("STRIP", "llvm-strip".to_string());
        h.insert("AS", format!("clang{target_flag}"));
        h.insert("CC", format!("clang{target_flag}"));
        h.insert("CXX", format!("clang++{target_flag}{target_cxxflag}"));
        h.insert("PKG_CONFIG", format!("{target_prefix}pkg-config"));
    }
    if is_host {
        for (k, v) in h.iter_mut() {
            if let Ok(env) = std::env::var(format!("REDOXER_HOST_{k}"))
                && !env.is_empty()
            {
                *v = env;
            }
        }
    }
    h
}

pub fn main(args: &[String]) {
    match inner(args.get(1).unwrap(), args.iter().skip(2).cloned()) {
        Ok(()) => {
            process::exit(0);
        }
        Err(err) => {
            eprintln!("redoxer env: {err:#}");
            process::exit(1);
        }
    }
}
