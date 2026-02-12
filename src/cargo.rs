use std::process;

use crate::{status_error, target};

fn inner<I: Iterator<Item = String>>(mut args: I) -> anyhow::Result<()> {
    let command = args.next().unwrap();
    let subcommand = args.next().unwrap();

    #[cfg(feature = "cli-exec")]
    let (runner, arguments) = {
        use anyhow::Context;
        let mut runner_config = crate::exec::RedoxerExecConfig::new(args)
            .context("Unable to parse exec configuration")?;
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
        (runner.join(" "), arguments)
    };
    #[cfg(not(feature = "cli-exec"))]
    let (runner, arguments) = {
        let mut matching = true;
        let mut arguments = Vec::new();
        while let Some(arg) = args.next() {
            match (arg.as_str(), matching) {
                (
                    "-f" | "--folder" | "-a" | "--artifact" | "-i" | "--install-config" | "-o"
                    | "--output" | "-g" | "--gui" | "-h" | "--help",
                    true,
                ) => anyhow::bail!("feature 'cli-exec' is not compiled, please omit exec args"),
                ("--", true) => matching = false,
                _ => {
                    matching = false;
                    arguments.push(arg);
                }
            }
        }
        (format!("{command} exec"), arguments)
    };

    let cc_target_var = target().replace("-", "_");
    let cargo_target_var = cc_target_var.to_uppercase();

    crate::env::command("cargo")?
        .arg(subcommand)
        .arg("--target")
        .arg(target())
        .args(arguments)
        .env(format!("CARGO_TARGET_{}_RUNNER", cargo_target_var), runner)
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
