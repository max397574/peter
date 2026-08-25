use crate::component::{Color, Component, Context, Dynamic, LuaAnnotated, Segment};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Clone, Serialize, LuaAnnotated)]
pub struct GitData {
    /// Current branch name
    pub branch: String,
    /// Short commit hash of HEAD
    pub commit_hash: String,
    /// Current special state: "REBASING", "MERGING", "CHERRY-PICKING", "BISECTING", or "REVERTING"
    pub repo_state: Option<String>,
    /// Commits ahead of upstream branch
    pub ahead: u32,
    /// Commits behind upstream branch
    pub behind: u32,
    /// Lines added
    pub insertions: u32,
    /// Lines removed
    pub deletions: u32,
    /// Number of files with changes vs HEAD
    pub files_changed: u32,
}

fn git_dir() -> Option<PathBuf> {
    let output = Command::new("git")
        .args(["rev-parse", "--git-dir"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
    Some(PathBuf::from(path))
}

fn git_repo_exists() -> bool {
    git_dir().is_some()
}

fn detect_repo_state(git_dir: &Path) -> Option<String> {
    if git_dir.join("rebase-merge").is_dir() || git_dir.join("rebase-apply").is_dir() {
        Some("REBASING".to_string())
    } else if git_dir.join("MERGE_HEAD").is_file() {
        Some("MERGING".to_string())
    } else if git_dir.join("CHERRY_PICK_HEAD").is_file() {
        Some("CHERRY-PICKING".to_string())
    } else if git_dir.join("BISECT_LOG").is_file() {
        Some("BISECTING".to_string())
    } else if git_dir.join("REVERT_HEAD").is_file() {
        Some("REVERTING".to_string())
    } else {
        None
    }
}

fn parse_branch_status(output: &str) -> (String, String, u32, u32) {
    let mut branch = String::new();
    let mut commit_hash = String::new();
    let mut ahead = 0;
    let mut behind = 0;

    for line in output.lines() {
        if let Some(rest) = line.strip_prefix("# branch.oid ") {
            commit_hash = if rest == "(initial)" {
                String::new()
            } else {
                rest.chars().take(7).collect()
            };
        } else if let Some(rest) = line.strip_prefix("# branch.head ") {
            branch = rest.to_string();
        } else if let Some(rest) = line.strip_prefix("# branch.ab ") {
            for token in rest.split_whitespace() {
                if let Some(n) = token.strip_prefix('+') {
                    ahead = n.parse().unwrap_or(0);
                } else if let Some(n) = token.strip_prefix('-') {
                    behind = n.parse().unwrap_or(0);
                }
            }
        }
    }

    (branch, commit_hash, ahead, behind)
}

fn parse_shortstat(output: &str) -> (u32, u32, u32) {
    let mut files_changed = 0;
    let mut insertions = 0;
    let mut deletions = 0;

    for part in output.trim().split(", ") {
        let Some(number_str) = part.split_whitespace().next() else {
            continue;
        };
        let Ok(number) = number_str.parse::<u32>() else {
            continue;
        };

        if part.contains("file") {
            files_changed = number;
        } else if part.contains("insertion") {
            insertions = number;
        } else if part.contains("deletion") {
            deletions = number;
        }
    }

    (files_changed, insertions, deletions)
}

fn get_git_data(_ctx: &Context, _config: &()) -> Option<GitData> {
    let git_dir_path = git_dir()?;

    let repo_state = detect_repo_state(&git_dir_path);

    let status_output = Command::new("git")
        .args([
            "status",
            "--porcelain=v2",
            "--branch",
            "--ignore-submodules",
        ])
        .output()
        .ok()?;
    if !status_output.status.success() {
        return None;
    }
    let status_text = String::from_utf8_lossy(&status_output.stdout);
    let (mut branch, commit_hash, ahead, behind) = parse_branch_status(&status_text);

    if branch.is_empty() || branch == "(detached)" {
        branch = commit_hash.clone();
    }

    let shortstat_output = Command::new("git")
        .args(["diff", "--shortstat", "HEAD"])
        .output()
        .ok();
    let (files_changed, insertions, deletions) = match shortstat_output {
        Some(output) if output.status.success() => {
            parse_shortstat(&String::from_utf8_lossy(&output.stdout))
        }
        _ => (0, 0, 0),
    };

    Some(GitData {
        branch,
        commit_hash,
        repo_state,
        ahead,
        behind,
        insertions,
        deletions,
        files_changed,
    })
}

fn render_git(data: &Option<GitData>) -> Vec<Segment> {
    let Some(data) = data else {
        return Vec::new();
    };

    let c_blue = Color::from_hex("#5FAFFF").unwrap();
    let c_purple = Color::from_hex("#AF87D7").unwrap();
    let c_gray = Color::from_hex("#BCBCBC").unwrap();
    let c_dark = Color::from_hex("#444444").unwrap();
    let c_green = Color::from_hex("#5FAF5F").unwrap();
    let c_red = Color::from_hex("#FF5F5F").unwrap();

    let mut segs = vec![Segment::new(" ", c_blue)];

    if let Some(state) = &data.repo_state {
        segs.push(Segment::new(format!("{state} "), c_red));
    }

    segs.push(Segment::new(format!(" {}", data.branch.clone()), c_green));
    if !data.commit_hash.is_empty() {
        segs.push(Segment::new(format!(" {}", data.commit_hash), c_gray));
    }

    if data.ahead > 0 {
        segs.push(Segment::new(format!(" \u{2191}{}", data.ahead), c_purple));
    }
    if data.behind > 0 {
        segs.push(Segment::new(format!(" \u{2193}{}", data.behind), c_purple));
    }

    if data.files_changed > 0 {
        segs.push(Segment::new(" |  ", c_dark));
        if data.insertions > 0 {
            segs.push(Segment::new(format!("+{} ", data.insertions), c_green));
        }
        if data.deletions > 0 {
            segs.push(Segment::new(format!("-{}", data.deletions), c_red));
        }
    }

    segs
}

pub fn component() -> Component<Option<GitData>, ()> {
    let mut c = Component::new("git", get_git_data, render_git);
    c.enabled = Dynamic::new_dyn(|_ctx| git_repo_exists());
    c
}
