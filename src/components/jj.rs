use crate::component::{Color, Component, Context, Dynamic, Segment};
use serde::Serialize;
use std::process::Command;

#[derive(Clone, Serialize)]
pub struct JJData {
    pub change_id: String,
    pub files_added: u32,
    pub files_modified: u32,
    pub files_deleted: u32,
    pub files_conflict: u32,
    pub description: String,
}

fn jj_root_exists() -> bool {
    Command::new("jj")
        .args(["--ignore-working-copy", "root"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn get_jj_data(_ctx: &Context, _config: &()) -> Option<JJData> {
    if !jj_root_exists() {
        return None;
    }

    let diff_output = Command::new("jj")
        .args(["diff", "--summary", "--ignore-working-copy"])
        .output();

    let mut files_added = 0;
    let mut files_modified = 0;
    let mut files_deleted = 0;
    let mut files_conflict = 0;

    if let Ok(output) = diff_output {
        let stdout = String::from_utf8_lossy(&output.stdout);
        for line in stdout.lines() {
            if line.starts_with("A ") {
                files_added += 1;
            } else if line.starts_with("M ") {
                files_modified += 1;
            } else if line.starts_with("D ") {
                files_deleted += 1;
            } else if line.starts_with("C ") {
                files_conflict += 1;
            }
        }
    }

    let log_output = Command::new("jj")
        .args([
            "log",
            "--revisions",
            "@",
            "--no-graph",
            "--ignore-working-copy",
            "--limit",
            "1",
            "--template",
            r#"separate("\n", change_id.shortest(4), description)"#,
        ])
        .output();

    let mut change_id = String::from("????");
    let mut description = String::from("(no description set)");

    if let Ok(output) = log_output {
        let stdout = String::from_utf8_lossy(&output.stdout);
        let lines: Vec<&str> = stdout.lines().collect();
        if let Some(id) = lines.first() {
            let trimmed = id.trim();
            if !trimmed.is_empty() {
                change_id = trimmed.to_string();
            }
        }
        if lines.len() > 1 {
            let desc = lines[1..].join("\n").trim().to_string();
            if !desc.is_empty() {
                description = desc;
            }
        }
    }

    Some(JJData {
        change_id,
        files_added,
        files_modified,
        files_deleted,
        files_conflict,
        description,
    })
}

fn render_jj(data: &Option<JJData>) -> Vec<Segment> {
    let Some(data) = data else {
        return Vec::new();
    };

    let c_blue = Color::from_hex("#5FAFFF").unwrap();
    let c_purple = Color::from_hex("#AF87D7").unwrap();
    let c_magenta = Color::from_hex("#D787D7").unwrap();
    let c_gray = Color::from_hex("#BCBCBC").unwrap();
    let c_dark = Color::from_hex("#444444").unwrap();
    let c_green = Color::from_hex("#5FAF5F").unwrap();

    let mut segs = vec![Segment::new("jj  ", c_blue)];

    if data.files_modified > 0 {
        segs.push(Segment::new(format!("!{} ", data.files_modified), c_purple));
    }
    if data.files_added > 0 {
        segs.push(Segment::new(format!("+{} ", data.files_added), c_purple));
    }
    if data.files_deleted > 0 {
        segs.push(Segment::new(format!("-{} ", data.files_deleted), c_purple));
    }
    if data.files_conflict > 0 {
        segs.push(Segment::new(format!("={} ", data.files_conflict), c_purple));
    }

    let (prefix, rest) = data.change_id.split_at(1.min(data.change_id.len()));
    segs.push(Segment::new(prefix.to_string(), c_magenta));
    segs.push(Segment::new(format!("{rest} "), c_gray));

    segs.push(Segment::new("|  ", c_dark));

    let first_line = data.description.lines().next().unwrap_or(&data.description);
    segs.push(Segment::new(first_line.to_string(), c_green));

    segs
}

pub fn component() -> Component<Option<JJData>, ()> {
    let mut c = Component::new("jj", get_jj_data, render_jj);
    c.enabled = Dynamic::new_dyn(|_ctx| jj_root_exists());
    c
}
