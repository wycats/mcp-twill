use std::{env, path::PathBuf};

use anyhow::{Context, Result, bail};

fn main() -> Result<()> {
    let mut args = env::args().skip(1);
    match args.next().as_deref() {
        Some("import-mcp-task-fixture") => {
            let mut core_repository = None;
            let mut extension_repository = None;
            let mut final_reference = None;
            let mut check = false;
            while let Some(argument) = args.next() {
                match argument.as_str() {
                    "--core-repository" => {
                        core_repository = Some(PathBuf::from(
                            args.next().context("--core-repository requires a path")?,
                        ));
                    }
                    "--extension-repository" => {
                        extension_repository = Some(PathBuf::from(
                            args.next()
                                .context("--extension-repository requires a path")?,
                        ));
                    }
                    "--final-ref" => {
                        final_reference =
                            Some(args.next().context("--final-ref requires a value")?);
                    }
                    "--check" => check = true,
                    other => bail!("unknown import-mcp-task-fixture argument `{other}`"),
                }
            }
            xtask::mcp_task_fixture::import(
                &core_repository.context("--core-repository is required")?,
                &extension_repository.context("--extension-repository is required")?,
                final_reference.as_deref(),
                check,
            )
        }
        Some("validate-mcp-task-release") => {
            if let Some(other) = args.next() {
                bail!("unknown validate-mcp-task-release argument `{other}`");
            }
            xtask::mcp_task_fixture::validate_release()
        }
        Some("import-vbl-fixture") => {
            let mut repository = None;
            let mut reference = None;
            let mut check = false;
            while let Some(argument) = args.next() {
                match argument.as_str() {
                    "--repository" => {
                        repository = Some(PathBuf::from(
                            args.next().context("--repository requires a path")?,
                        ));
                    }
                    "--ref" => {
                        reference = Some(args.next().context("--ref requires a value")?);
                    }
                    "--check" => check = true,
                    other => bail!("unknown import-vbl-fixture argument `{other}`"),
                }
            }
            let repository = repository.context("--repository is required")?;
            let reference = reference.context("--ref is required")?;
            xtask::vbl_fixture::import(&repository, &reference, check)
        }
        Some(other) => bail!("unknown xtask command `{other}`"),
        None => bail!(
            "usage: cargo xtask <import-vbl-fixture|import-mcp-task-fixture|validate-mcp-task-release> ..."
        ),
    }
}
