// todokase.rs — read a live task list from the sibling task app (todokase /
// todarchy) for embedding in a note.
//
// The task app maintains a derived JSON view at ~/.local/share/todarchy/
// tasks.json (kept in step with its CRDT). We just read and filter it — no
// dependency on its crate, no Automerge, matching the "notes are just files,
// read another app's file" philosophy. Override the path with
// NOTAKASE_TODOKASE_JSON (used by tests).

use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::{Context, Result};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq)]
pub enum Status {
    Open,
    Done,
    All,
}

impl Status {
    fn parse(s: &str) -> Status {
        match s.trim().to_lowercase().as_str() {
            "done" | "completed" | "closed" => Status::Done,
            "all" | "any" => Status::All,
            _ => Status::Open,
        }
    }
}

/// A parsed embed query (the body of a ```todokase fenced block).
#[derive(Debug, Clone)]
pub struct Query {
    /// Match a project by name (case-insensitive) or id.
    pub project: Option<String>,
    /// Match a context (with or without the leading `@`).
    pub context: Option<String>,
    pub status: Status,
    pub limit: usize,
}

impl Default for Query {
    fn default() -> Self {
        Query { project: None, context: None, status: Status::Open, limit: 100 }
    }
}

/// One task ready to render.
#[derive(Debug, Clone, PartialEq)]
pub struct TaskItem {
    pub id: String,
    pub title: String,
    pub done: bool,
    pub ctx: Option<String>,
    pub due: Option<String>,
    pub project: Option<String>,
}

/// Parse the `key: value` lines of a todokase embed block.
pub fn parse_query(body: &str) -> Query {
    let mut q = Query::default();
    for line in body.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((k, v)) = line.split_once(':') else { continue };
        let key = k.trim().to_lowercase();
        let val = v.trim().to_string();
        match key.as_str() {
            "project" | "list" => q.project = (!val.is_empty()).then_some(val),
            "context" | "ctx" => q.context = (!val.is_empty()).then_some(val),
            "status" => q.status = Status::parse(&val),
            "limit" => {
                if let Ok(n) = val.parse::<usize>() {
                    q.limit = n;
                }
            }
            _ => {}
        }
    }
    q
}

/// The task app's derived JSON view.
pub fn tasks_json_path() -> PathBuf {
    if let Some(p) = std::env::var_os("NOTAKASE_TODOKASE_JSON") {
        return PathBuf::from(p);
    }
    let data = dirs::data_dir().unwrap_or_else(|| {
        dirs::home_dir().unwrap_or_else(|| PathBuf::from(".")).join(".local/share")
    });
    data.join("todarchy/tasks.json")
}

/// Load and parse the task app's tasks.json.
pub fn load_doc() -> Result<Value> {
    let path = tasks_json_path();
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("reading {}", path.display()))?;
    serde_json::from_str(&text).context("parsing tasks.json")
}

/// Run a query against the live task list.
pub fn query(q: &Query) -> Result<Vec<TaskItem>> {
    Ok(filter(&load_doc()?, q))
}

/// Every task referenced by the `todokase` embed blocks in a note (deduped by
/// id), reading the task list once. Empty if the note has no embeds.
pub fn tasks_in_note(md: &str) -> Result<Vec<TaskItem>> {
    let blocks = todokase_blocks(md);
    if blocks.is_empty() {
        return Ok(Vec::new());
    }
    let doc = load_doc()?;
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for body in blocks {
        for t in filter(&doc, &parse_query(&body)) {
            if seen.insert(t.id.clone()) {
                out.push(t);
            }
        }
    }
    Ok(out)
}

/// Extract the bodies of fenced ```todokase blocks from markdown.
fn todokase_blocks(md: &str) -> Vec<String> {
    let mut blocks = Vec::new();
    let mut in_block = false;
    let mut buf = String::new();
    for line in md.lines() {
        let t = line.trim_start();
        if !in_block {
            let fence = t.strip_prefix("```").or_else(|| t.strip_prefix("~~~"));
            if let Some(info) = fence {
                if info.trim().eq_ignore_ascii_case("todokase") {
                    in_block = true;
                    buf.clear();
                }
            }
        } else if t.starts_with("```") || t.starts_with("~~~") {
            in_block = false;
            blocks.push(std::mem::take(&mut buf));
        } else {
            buf.push_str(line);
            buf.push('\n');
        }
    }
    blocks
}

/// Pure filter over a parsed tasks.json document (split out for testing).
pub fn filter(doc: &Value, q: &Query) -> Vec<TaskItem> {
    // project id -> name
    let mut names: HashMap<String, String> = HashMap::new();
    if let Some(arr) = doc.get("projects").and_then(Value::as_array) {
        for p in arr {
            if let (Some(id), Some(name)) =
                (p.get("id").and_then(Value::as_str), p.get("name").and_then(Value::as_str))
            {
                names.insert(id.to_string(), name.to_string());
            }
        }
    }

    let want_ctx = q.context.as_deref().map(|c| c.trim_start_matches('@').to_lowercase());
    let mut out: Vec<(f64, TaskItem)> = Vec::new();

    if let Some(arr) = doc.get("tasks").and_then(Value::as_array) {
        for t in arr {
            let title = t.get("title").and_then(Value::as_str).unwrap_or("").trim().to_string();
            if title.is_empty() {
                continue;
            }
            let list = t.get("list").and_then(Value::as_str).unwrap_or("");
            let project = names.get(list).cloned();
            let ctx = str_field(t, "ctx");
            let due = str_field(t, "due");
            let done = match t.get("doneAt") {
                None | Some(Value::Null) => false,
                Some(Value::String(s)) => !s.is_empty(),
                Some(_) => true,
            };

            // filters
            if let Some(qp) = &q.project {
                let matches = list.eq_ignore_ascii_case(qp)
                    || project.as_deref().map(|n| n.eq_ignore_ascii_case(qp)).unwrap_or(false);
                if !matches {
                    continue;
                }
            }
            if let Some(wc) = &want_ctx {
                let ok = ctx
                    .as_deref()
                    .map(|c| c.trim_start_matches('@').eq_ignore_ascii_case(wc))
                    .unwrap_or(false);
                if !ok {
                    continue;
                }
            }
            match q.status {
                Status::Open if done => continue,
                Status::Done if !done => continue,
                _ => {}
            }

            let id = t.get("id").and_then(Value::as_str).unwrap_or("").to_string();
            let pos = t
                .get("pos")
                .and_then(|v| v.as_str().and_then(|s| s.parse().ok()).or_else(|| v.as_f64()))
                .unwrap_or(0.0);
            out.push((pos, TaskItem { id, title, done, ctx, due, project }));
        }
    }

    out.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    out.into_iter().take(q.limit).map(|(_, t)| t).collect()
}

fn str_field(t: &Value, key: &str) -> Option<String> {
    t.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn doc() -> Value {
        json!({
            "projects": [
                {"id": "p_work", "name": "Work"},
                {"id": "p_home", "name": "Home"}
            ],
            "tasks": [
                {"id": "t1", "title": "ship v1", "list": "p_work", "ctx": "@work", "due": "today", "pos": "2", "doneAt": "123"},
                {"id": "t2", "title": "write spec", "list": "p_work", "ctx": "", "due": "", "pos": "1"},
                {"id": "t3", "title": "buy milk", "list": "p_home", "ctx": "@errands", "due": "", "pos": "3"}
            ]
        })
    }

    #[test]
    fn parse_query_reads_fields() {
        let q = parse_query("project: Work\nstatus: all\ncontext: @work\nlimit: 5");
        assert_eq!(q.project.as_deref(), Some("Work"));
        assert_eq!(q.status, Status::All);
        assert_eq!(q.context.as_deref(), Some("@work"));
        assert_eq!(q.limit, 5);
        // defaults
        let d = parse_query("");
        assert_eq!(d.status, Status::Open);
        assert!(d.project.is_none());
    }

    #[test]
    fn filter_by_project_and_open_status_default() {
        let q = parse_query("project: Work"); // status defaults to Open
        let tasks = filter(&doc(), &q);
        // only the open Work task; the done one ("ship v1") is excluded
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].title, "write spec");
        assert_eq!(tasks[0].project.as_deref(), Some("Work"));
    }

    #[test]
    fn status_all_includes_done_sorted_by_pos() {
        let q = parse_query("project: Work\nstatus: all");
        let tasks = filter(&doc(), &q);
        assert_eq!(tasks.len(), 2);
        // pos "1" (write spec) sorts before pos "2" (ship v1)
        assert_eq!(tasks[0].title, "write spec");
        assert_eq!(tasks[1].title, "ship v1");
        assert!(tasks[1].done);
    }

    #[test]
    fn filter_by_context() {
        let q = parse_query("context: errands\nstatus: all");
        let tasks = filter(&doc(), &q);
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].title, "buy milk");
        assert_eq!(tasks[0].id, "t3");
    }

    #[test]
    fn extracts_todokase_blocks_from_markdown() {
        let md = "# Note\n\ntext\n\n```todokase\nproject: Work\n```\n\nmore\n\n\
                  ```rust\nfn main() {}\n```\n\n~~~todokase\ncontext: @home\n~~~\n";
        let blocks = todokase_blocks(md);
        assert_eq!(blocks.len(), 2, "should find both todokase fences, not the rust one");
        assert!(blocks[0].contains("project: Work"));
        assert!(blocks[1].contains("context: @home"));
    }
}
