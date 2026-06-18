//! Shared git change-set detection, used by `bdo review` and `bdo map --changed`.
//!
//! All git output is read NUL-terminated (`-z`): otherwise git C-quotes paths
//! with spaces/non-ASCII, which breaks on-disk reads and path matching. `-z`
//! emits raw, unquoted paths and lets rename records be parsed unambiguously.

use anyhow::{Context, Result};
use std::process::Command;

/// A changed path with its git status code (e.g. "M", "A", "??", "R").
pub struct Change {
    pub status: String,
    pub path: String,
}

/// Are we inside a git work tree?
pub fn in_git_repo() -> bool {
    Command::new("git")
        .args(["rev-parse", "--is-inside-work-tree"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn git_stdout(args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .args(args)
        .output()
        .context("failed to run git")?;
    if !output.status.success() {
        anyhow::bail!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// The change set: working-tree changes (incl. untracked) when `against` is
/// `None`, or the diff vs a ref (plus untracked files) when `against` is `Some`.
pub fn changed_files(against: Option<&str>) -> Result<Vec<Change>> {
    match against {
        Some(base) => {
            let mut changes =
                parse_name_status_z(&git_stdout(&["diff", "--name-status", "-z", base])?);
            // `git diff` only sees tracked changes; add untracked files so new
            // files are still considered in --against mode.
            let untracked = git_stdout(&["ls-files", "--others", "--exclude-standard", "-z"])?;
            for path in untracked.split('\0').filter(|p| !p.is_empty()) {
                changes.push(Change {
                    status: "??".to_string(),
                    path: path.to_string(),
                });
            }
            Ok(changes)
        }
        None => Ok(parse_porcelain_z(&git_stdout(&[
            "status",
            "--porcelain=v1",
            "-z",
        ])?)),
    }
}

/// Parse `git diff --name-status -z` output: each record is `STATUS\0PATH\0`,
/// and for a rename/copy `R###\0SRC\0DST\0` — keep the new (DST) path.
fn parse_name_status_z(raw: &str) -> Vec<Change> {
    let mut out = Vec::new();
    let mut tokens = raw.split('\0').filter(|t| !t.is_empty());
    while let Some(status) = tokens.next() {
        let st = status.chars().next().unwrap_or('?');
        let path = if status.starts_with('R') || status.starts_with('C') {
            let _src = tokens.next();
            tokens.next() // DST is the current path
        } else {
            tokens.next()
        };
        let Some(path) = path else { break };
        out.push(Change {
            status: st.to_string(),
            path: path.to_string(),
        });
    }
    out
}

/// Parse `git status --porcelain=v1 -z` output. Each record is `XY PATH` (XY =
/// 2 status cols, then a space, then the path). For a rename/copy the original
/// path follows as a separate NUL field (`XY NEW\0ORIG\0`) — keep NEW, drop ORIG.
fn parse_porcelain_z(raw: &str) -> Vec<Change> {
    let mut out = Vec::new();
    let mut tokens = raw.split('\0');
    while let Some(tok) = tokens.next() {
        if tok.len() < 3 {
            continue; // trailing empty token, etc.
        }
        let status = tok[..2].trim().to_string();
        let path = tok[3..].to_string();
        if status.starts_with('R') || status.starts_with('C') {
            let _ = tokens.next(); // consume & drop the ORIG path field
        }
        out.push(Change { status, path });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_porcelain_z_statuses_and_paths() {
        // NUL-terminated records; a path with a space must survive verbatim.
        let raw = " M src/core/filter.rs\0?? new file.txt\0A  staged.rs\0";
        let c = parse_porcelain_z(raw);
        assert_eq!(c.len(), 3);
        assert_eq!(c[0].status, "M");
        assert_eq!(c[0].path, "src/core/filter.rs");
        assert_eq!(c[1].status, "??");
        assert_eq!(c[1].path, "new file.txt");
        assert_eq!(c[2].status, "A");
    }

    #[test]
    fn test_parse_porcelain_z_rename_keeps_new_drops_orig() {
        // `git status --porcelain=v1 -z`: `XY NEW\0ORIG\0` (new first).
        let raw = "R  new name.txt\0old name.txt\0";
        let c = parse_porcelain_z(raw);
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].status, "R");
        assert_eq!(c[0].path, "new name.txt");
    }

    #[test]
    fn test_parse_name_status_z_rename_takes_dst() {
        // `git diff --name-status -z`: `STATUS\0[SRC\0]DST\0` (DST is current).
        let raw = "M\0src/main.rs\0R066\0old name.txt\0new name.txt\0A\0x.rs\0";
        let c = parse_name_status_z(raw);
        assert_eq!(c.len(), 3);
        assert_eq!(c[0].status, "M");
        assert_eq!(c[0].path, "src/main.rs");
        assert_eq!(c[1].status, "R");
        assert_eq!(c[1].path, "new name.txt");
        assert_eq!(c[2].status, "A");
        assert_eq!(c[2].path, "x.rs");
    }
}
