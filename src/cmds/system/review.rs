//! `bdo review` — a one-shot change summary for human + agent review.
//!
//! Combines what you'd otherwise assemble by hand after editing: the changed
//! file list, generated artifacts that shouldn't be committed, stale markers
//! (legacy names / broken install URLs), and the unit tests worth running.
//! Scoped to the change set (working tree, or `--against <ref>`), not the whole
//! repo — keeping the output small, in the spirit of the rest of bdo.

use crate::core::changes::{changed_files, in_git_repo, rust_test_targets};
use crate::core::tracking;
use anyhow::Result;

/// Generated/junk path fragments that usually should not be committed.
/// `(fragment, label)` — `fragment` is matched as a substring of the path.
const ARTIFACT_MARKERS: &[(&str, &str)] = &[
    ("__pycache__/", "python bytecode dir"),
    (".pyc", "python bytecode"),
    ("target/", "cargo build output"),
    (".DS_Store", "macOS metadata"),
    ("node_modules/", "node dependencies"),
    (".orig", "merge leftover"),
    (".rej", "patch reject"),
    (".bak", "backup file"),
];

/// High-signal stale strings that are almost always a mistake in this repo.
/// Built with `concat!` so the patterns are not contiguous in this source file
/// (otherwise `bdo review` would flag its own implementation).
fn stale_markers() -> Vec<(String, &'static str)> {
    vec![
        (concat!("cargo install ", "bdo").to_string(), "wrong crate name (use --git or `bushido`)"),
        (concat!("rtk", "-rewrite").to_string(), "legacy hook script name"),
        (concat!("rtk", "-hook-version").to_string(), "legacy hook version marker"),
        (concat!("rtk", "-awareness").to_string(), "legacy awareness file name"),
        (concat!(".config/", "rtk").to_string(), "legacy config dir"),
        (concat!("blob/", "master").to_string(), "broken install URL (blob serves HTML)"),
        (concat!("feat/", "all-features").to_string(), "obsolete fork branch"),
    ]
}

pub fn run(against: Option<&str>, verbose: u8) -> Result<()> {
    let timer = tracking::TimedExecution::start();

    if !in_git_repo() {
        anyhow::bail!("bdo review: not inside a git repository");
    }

    let changes = changed_files(against, None)?;
    let base_label = against.unwrap_or("uncommitted");

    let mut out = String::new();
    out.push_str(&format!(
        "bdo review — {} changed file(s) ({})\n",
        changes.len(),
        base_label
    ));

    if changes.is_empty() {
        out.push_str("\n✓ no changes to review\n");
        print!("{}", out);
        timer.track("review", "bdo review", "", &out);
        return Ok(());
    }

    // ── Changed files ────────────────────────────────────────────
    out.push_str("\nCHANGED\n");
    for c in &changes {
        out.push_str(&format!("  {:<2} {}\n", c.status, c.path));
    }

    // ── Suspicious artifacts ─────────────────────────────────────
    let artifacts: Vec<(&str, &str)> = changes
        .iter()
        .filter(|c| c.status != "D")
        .filter_map(|c| artifact_reason(&c.path).map(|r| (c.path.as_str(), r)))
        .collect();
    out.push_str(&section_header("⚠ ARTIFACTS", artifacts.len(), "likely should not be committed"));
    for (path, reason) in &artifacts {
        out.push_str(&format!("  {}  [{}]\n", path, reason));
    }

    // ── Stale markers (scan changed, non-deleted text files) ─────
    let markers = stale_markers();
    let mut stale_hits: Vec<String> = Vec::new();
    for c in &changes {
        if c.status == "D" {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(&c.path) else {
            continue; // missing or binary
        };
        for (lineno, line) in content.lines().enumerate() {
            for (pat, label) in &markers {
                if line.contains(pat.as_str()) {
                    stale_hits.push(format!("  {}:{}  {}", c.path, lineno + 1, label));
                    break; // one hit per line is enough
                }
            }
            if stale_hits.len() >= 40 {
                break;
            }
        }
        if stale_hits.len() >= 40 {
            stale_hits.push("  … (more; capped at 40)".to_string());
            break;
        }
    }
    out.push_str(&section_header("⚠ STALE MARKERS", stale_hits.len(), "verify before commit"));
    for hit in &stale_hits {
        out.push_str(hit);
        out.push('\n');
    }

    // ── Suggested tests ──────────────────────────────────────────
    let targets = rust_test_targets(&changes);
    if targets.is_empty() {
        out.push_str("\n🧪 SUGGESTED TESTS\n  ✓ none (no Rust sources changed)\n");
    } else {
        out.push_str("\n🧪 SUGGESTED TESTS\n");
        // Multiple filters must follow `--` (libtest ORs them); bare positional
        // filters are rejected by cargo.
        out.push_str(&format!(
            "  cargo test -- {}\n",
            targets.iter().cloned().collect::<Vec<_>>().join(" ")
        ));
    }

    print!("{}", out);
    if verbose > 0 {
        eprintln!("reviewed {} changed files vs {}", changes.len(), base_label);
    }
    timer.track("review", "bdo review", "", &out);
    Ok(())
}

fn section_header(title: &str, count: usize, hint: &str) -> String {
    if count == 0 {
        format!("\n{} (0)\n  ✓ none\n", title)
    } else {
        format!("\n{} ({}) — {}\n", title, count, hint)
    }
}

fn artifact_reason(path: &str) -> Option<&'static str> {
    ARTIFACT_MARKERS
        .iter()
        .find(|(frag, _)| {
            if let Some(dir) = frag.strip_suffix('/') {
                // Directory marker: match only as a full path segment, so
                // `mytarget/x` doesn't trip the `target/` rule.
                path.starts_with(frag) || path.contains(&format!("/{dir}/"))
            } else {
                // Suffix/substring marker (.pyc, .DS_Store, .bak, …).
                path.contains(frag)
            }
        })
        .map(|(_, label)| *label)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_artifact_reason() {
        assert!(artifact_reason("a/__pycache__/x.pyc").is_some());
        assert!(artifact_reason("target/debug/bdo").is_some());
        assert!(artifact_reason("src/core/filter.rs").is_none());
    }

    #[test]
    fn test_stale_markers_detect_known_bad_strings() {
        let markers = stale_markers();
        let hit = |s: &str| markers.iter().any(|(p, _)| s.contains(p.as_str()));
        // Split literals with concat! so `bdo review` doesn't flag this fixture.
        assert!(hit(concat!("run: cargo install ", "bdo")));
        assert!(hit(concat!("see hooks/claude/rtk", "-rewrite.sh")));
        assert!(hit(concat!("curl .../blob/", "master/install.sh")));
        assert!(!hit("a perfectly normal line"));
    }
}
