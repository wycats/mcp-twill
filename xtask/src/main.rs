use std::{env, path::PathBuf};

use anyhow::{Context, Result, bail};

fn main() -> Result<()> {
    let mut args = env::args().skip(1);
    match args.next().as_deref() {
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
            "usage: cargo xtask import-vbl-fixture --repository <local-git-repository> --ref v0.4.9 [--check]"
        ),
    }
}
