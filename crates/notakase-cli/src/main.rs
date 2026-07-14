// nota — CLI companion to the notakase note vault.
//
// Drives the same notakase-core vault as the TUI and the MCP server, so notes
// created/edited here show up in the TUI and ride any configured sync.
//
//   nota list                         # all notes by path
//   nota read Projects/x/spec.md      # print a note's body
//   nota new  Journal/today.md "# Mon\n\nstarted the day"
//   nota append inbox.md "another thought"
//   nota search drums                 # path/body substring match
//   nota rm Projects/x/old.md

mod session;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};

use session::Session;

#[derive(Parser)]
#[command(name = "nota", version, about = "Omarchy-native notes — CLI companion to the notakase TUI")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// List every note in the vault by path.
    List,
    /// Print a note's markdown body.
    Read { path: String },
    /// Create a note at a vault path; remaining args become the body.
    New { path: String, body: Vec<String> },
    /// Append text to an existing note.
    Append { path: String, text: Vec<String> },
    /// Find notes whose path or body contains the query.
    Search { query: String },
    /// Delete a note by path.
    Rm { path: String },
}

#[tokio::main]
async fn main() -> Result<()> {
    match Cli::parse().cmd {
        Cmd::List => list().await,
        Cmd::Read { path } => read(&path).await,
        Cmd::New { path, body } => new(&path, &body.join(" ")).await,
        Cmd::Append { path, text } => append(&path, &text.join(" ")).await,
        Cmd::Search { query } => search(&query).await,
        Cmd::Rm { path } => rm(&path).await,
    }
}

async fn list() -> Result<()> {
    let session = Session::open().await?;
    let mut notes = session.vault.live_notes();
    notes.sort_by(|a, b| a.doc.path().cmp(&b.doc.path()));
    if notes.is_empty() {
        println!("(vault is empty)");
        return Ok(());
    }
    for n in notes {
        let path = n.doc.path();
        let title = notakase_core::store::title_of(&n.doc.body()).unwrap_or_default();
        if title.is_empty() || title == path {
            println!("· {path}");
        } else {
            println!("· {path}  —  {title}");
        }
    }
    Ok(())
}

async fn read(path: &str) -> Result<()> {
    let session = Session::open().await?;
    let note = session.vault.note_by_path(path).context(format!("no note at '{path}'"))?;
    println!("{}", note.doc.body());
    Ok(())
}

async fn new(path: &str, body: &str) -> Result<()> {
    let mut session = Session::open().await?;
    session.vault.create_note(path, body).context("create failed")?;
    session.sync().await;
    println!("✓ created {path}");
    Ok(())
}

async fn append(path: &str, text: &str) -> Result<()> {
    if text.trim().is_empty() {
        anyhow::bail!("nothing to append");
    }
    let mut session = Session::open().await?;
    let now = notakase_core::util::now_ms();
    let note = session
        .vault
        .notes
        .iter_mut()
        .find(|n| !n.doc.deleted() && n.doc.path() == path)
        .context(format!("no note at '{path}'"))?;
    let combined = format!("{}\n\n{}", note.doc.body().trim_end(), text);
    note.doc.set_body(&combined, now).context("append failed")?;
    session.vault.persist().context("persist failed")?;
    session.vault.materialize().context("materialize failed")?;
    session.sync().await;
    println!("✓ appended to {path}");
    Ok(())
}

async fn search(query: &str) -> Result<()> {
    let q = query.to_lowercase();
    let session = Session::open().await?;
    let mut any = false;
    for n in session.vault.live_notes() {
        let path = n.doc.path();
        let body = n.doc.body();
        if path.to_lowercase().contains(&q) || body.to_lowercase().contains(&q) {
            any = true;
            println!("· {path}");
        }
    }
    if !any {
        println!("(no notes match '{query}')");
    }
    Ok(())
}

async fn rm(path: &str) -> Result<()> {
    let mut session = Session::open().await?;
    session.vault.delete_note(path).context("delete failed")?;
    session.sync().await;
    println!("✓ deleted {path}");
    Ok(())
}
