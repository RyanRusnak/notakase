// tools.rs — the MCP tools, driving the notakase Vault. Notes are addressed by
// their vault path (e.g. "Projects/todokase/spec.md"). Reads pull the latest;
// writes push through the session's sync transports.

use serde_json::{json, Value};

use crate::session::Session;

pub enum ToolError {
    Unknown,
    Failed(String),
}

fn fail(msg: impl Into<String>) -> ToolError {
    ToolError::Failed(msg.into())
}

pub fn list() -> Vec<Value> {
    vec![
        json!({
            "name": "list_notes",
            "description": "List every note in the vault by path, with its title and last-modified time.",
            "inputSchema": { "type": "object", "properties": {} }
        }),
        json!({
            "name": "read_note",
            "description": "Read a note's full markdown body.",
            "inputSchema": {
                "type": "object",
                "properties": { "path": { "type": "string", "description": "Vault path, e.g. Projects/x/spec.md" } },
                "required": ["path"]
            }
        }),
        json!({
            "name": "search_notes",
            "description": "Find notes whose path or body contains the query (case-insensitive).",
            "inputSchema": {
                "type": "object",
                "properties": { "query": { "type": "string" } },
                "required": ["query"]
            }
        }),
        json!({
            "name": "create_note",
            "description": "Create a new note at a vault path with markdown body. Use this to expand a task into a note. Folders in the path are created as needed.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Vault path ending in .md, e.g. Projects/x/plan.md" },
                    "body": { "type": "string", "description": "Markdown content." }
                },
                "required": ["path", "body"]
            }
        }),
        json!({
            "name": "append_to_note",
            "description": "Append markdown text to an existing note (separated by a blank line).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "text": { "type": "string" }
                },
                "required": ["path", "text"]
            }
        }),
        json!({
            "name": "delete_note",
            "description": "Delete a note by path.",
            "inputSchema": {
                "type": "object",
                "properties": { "path": { "type": "string" } },
                "required": ["path"]
            }
        }),
    ]
}

pub async fn call(name: &str, args: Value) -> Result<String, ToolError> {
    match name {
        "list_notes" => list_notes().await,
        "read_note" => read_note(args).await,
        "search_notes" => search_notes(args).await,
        "create_note" => create_note(args).await,
        "append_to_note" => append_to_note(args).await,
        "delete_note" => delete_note(args).await,
        _ => Err(ToolError::Unknown),
    }
}

fn arg_str(args: &Value, key: &str) -> Option<String> {
    args.get(key).and_then(Value::as_str).map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

async fn open() -> Result<Session, ToolError> {
    Session::open().await.map_err(|e| fail(format!("open vault failed: {e}")))
}

fn title_or_path(path: &str, body: &str) -> String {
    notakase_core::store::title_of(body).unwrap_or_else(|| path.to_string())
}

async fn list_notes() -> Result<String, ToolError> {
    let session = open().await?;
    let mut notes = session.vault.live_notes();
    notes.sort_by_key(|n| std::cmp::Reverse(n.doc.modified()));
    if notes.is_empty() {
        return Ok("(vault is empty)".into());
    }
    let lines: Vec<String> = notes
        .iter()
        .map(|n| {
            let path = n.doc.path();
            let title = title_or_path(&path, &n.doc.body());
            format!("{path}  —  {title}")
        })
        .collect();
    Ok(format!("{} note(s)\n{}", lines.len(), lines.join("\n")))
}

async fn read_note(args: Value) -> Result<String, ToolError> {
    let path = arg_str(&args, "path").ok_or_else(|| fail("path is required"))?;
    let session = open().await?;
    match session.vault.note_by_path(&path) {
        Some(n) => Ok(n.doc.body()),
        None => Err(fail(format!("no note at '{path}'"))),
    }
}

async fn search_notes(args: Value) -> Result<String, ToolError> {
    let q = arg_str(&args, "query").ok_or_else(|| fail("query is required"))?.to_lowercase();
    let session = open().await?;
    let mut hits: Vec<String> = Vec::new();
    for n in session.vault.live_notes() {
        let path = n.doc.path();
        let body = n.doc.body();
        if path.to_lowercase().contains(&q) || body.to_lowercase().contains(&q) {
            // first body line matching the query, for context
            let snippet = body
                .lines()
                .find(|l| l.to_lowercase().contains(&q))
                .map(|l| l.trim())
                .filter(|l| !l.is_empty())
                .unwrap_or("");
            hits.push(if snippet.is_empty() {
                path.clone()
            } else {
                format!("{path}  —  {snippet}")
            });
        }
    }
    if hits.is_empty() {
        Ok(format!("(no notes match '{q}')"))
    } else {
        Ok(format!("{} match(es)\n{}", hits.len(), hits.join("\n")))
    }
}

async fn create_note(args: Value) -> Result<String, ToolError> {
    let path = arg_str(&args, "path").ok_or_else(|| fail("path is required"))?;
    let body = args.get("body").and_then(Value::as_str).unwrap_or("").to_string();
    let mut session = open().await?;
    session
        .vault
        .create_note(&path, &body)
        .map_err(|e| fail(format!("create failed: {e}")))?;
    session.sync().await;
    Ok(format!("Created note {path}."))
}

async fn append_to_note(args: Value) -> Result<String, ToolError> {
    let path = arg_str(&args, "path").ok_or_else(|| fail("path is required"))?;
    let text = arg_str(&args, "text").ok_or_else(|| fail("text is required"))?;
    let mut session = open().await?;
    let now = notakase_core::util::now_ms();
    let note = session
        .vault
        .notes
        .iter_mut()
        .find(|n| !n.doc.deleted() && n.doc.path() == path)
        .ok_or_else(|| fail(format!("no note at '{path}'")))?;
    let combined = format!("{}\n\n{}", note.doc.body().trim_end(), text);
    note.doc.set_body(&combined, now).map_err(|e| fail(format!("append failed: {e}")))?;
    session.vault.persist().map_err(|e| fail(format!("persist failed: {e}")))?;
    session.vault.materialize().map_err(|e| fail(format!("materialize failed: {e}")))?;
    session.sync().await;
    Ok(format!("Appended to {path}."))
}

async fn delete_note(args: Value) -> Result<String, ToolError> {
    let path = arg_str(&args, "path").ok_or_else(|| fail("path is required"))?;
    let mut session = open().await?;
    session
        .vault
        .delete_note(&path)
        .map_err(|e| fail(format!("delete failed: {e}")))?;
    session.sync().await;
    Ok(format!("Deleted {path}."))
}
