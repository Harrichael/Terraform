//! Run a SCIP indexer over a project tree. The indexer is chosen from the
//! project's manifest; the tools themselves have to be on PATH, and their
//! progress output is passed straight through to the terminal because a
//! Rust index means a build and can take a while.

use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result, bail};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Language {
    Rust,
    TypeScript,
    Go,
}

impl Language {
    pub fn detect(root: &Path) -> Option<Language> {
        if root.join("Cargo.toml").is_file() {
            Some(Language::Rust)
        } else if root.join("go.mod").is_file() {
            Some(Language::Go)
        } else if root.join("tsconfig.json").is_file() || root.join("package.json").is_file() {
            Some(Language::TypeScript)
        } else {
            None
        }
    }

    pub fn tool(self) -> &'static str {
        match self {
            Language::Rust => "rust-analyzer",
            Language::TypeScript => "npx",
            Language::Go => "scip-go",
        }
    }

    fn install_hint(self) -> &'static str {
        match self {
            Language::Rust => "rustup component add rust-analyzer",
            Language::TypeScript => "install Node.js; npx fetches @sourcegraph/scip-typescript itself",
            Language::Go => "go install github.com/scip-code/scip-go/cmd/scip-go@latest",
        }
    }

    fn command(self, out: &Path) -> Command {
        let mut cmd = Command::new(self.tool());
        match self {
            Language::Rust => cmd.args(["scip", "."]).arg("--output").arg(out),
            Language::TypeScript => {
                cmd.args(["--yes", "@sourcegraph/scip-typescript", "index", "--output"]).arg(out)
            }
            Language::Go => cmd.arg("--output").arg(out),
        };
        cmd
    }
}

/// Index `root` into `out`, returning which indexer ran.
pub fn index_project(root: &Path, out: &Path) -> Result<Language> {
    let lang = Language::detect(root).with_context(|| {
        format!("cannot pick a SCIP indexer: no Cargo.toml, go.mod, tsconfig.json or package.json in {}", root.display())
    })?;
    // The indexer runs with `root` as its working directory, so a relative
    // output path would land inside the project.
    let out = std::path::absolute(out)?;
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let status = match lang.command(&out).current_dir(root).status() {
        Ok(status) => status,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            bail!("{} is not on PATH ({})", lang.tool(), lang.install_hint())
        }
        Err(e) => return Err(e).with_context(|| format!("running {}", lang.tool())),
    };
    if !status.success() {
        bail!("{} failed ({status}) while indexing {}", lang.tool(), root.display());
    }
    if !out.is_file() {
        bail!("{} exited successfully but wrote no index at {}", lang.tool(), out.display());
    }
    Ok(lang)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn language_follows_the_manifest_with_rust_and_go_taking_precedence() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(Language::detect(dir.path()), None);
        std::fs::write(dir.path().join("package.json"), "{}").unwrap();
        assert_eq!(Language::detect(dir.path()), Some(Language::TypeScript));
        std::fs::write(dir.path().join("go.mod"), "module x").unwrap();
        assert_eq!(Language::detect(dir.path()), Some(Language::Go));
        std::fs::write(dir.path().join("Cargo.toml"), "[package]").unwrap();
        assert_eq!(Language::detect(dir.path()), Some(Language::Rust));

        let err = index_project(&dir.path().join("nowhere"), &dir.path().join("out.scip")).unwrap_err();
        assert!(err.to_string().contains("cannot pick a SCIP indexer"), "{err}");
    }
}
