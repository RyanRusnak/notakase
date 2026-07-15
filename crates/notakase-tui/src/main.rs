// notakase — a keyboard-first, terminal-native notes browser for Omarchy.
//
// The left pane is a folder tree; the right pane is a themed markdown preview.
// Colors track the live Omarchy theme.
//
//   notakase [VAULT_DIR]
//
// The vault is a directory of plain markdown files. It defaults to
// $NOTAKASE_NOTES, then the bundled sample vault. On launch we open the vault
// through notakase-core, which folds any on-disk edits into the canonical
// per-note CRDT documents (~/.local/share/notakase) that back sync — then the
// TUI browses the materialized files. Toggle glyphs with NOTAKASE_ASCII=1,
// accent with NOTAKASE_ACCENT.

mod app;
mod markdown;
mod theme;
mod tree;
mod ui;
mod watch;

use std::io::{self, Stdout};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use notakase_core::cryptobox::KEY_BYTES;
use notakase_core::keystore::LibSecretKeyStore;
use notakase_core::{config, sharelink, vaultkey, Config, ServerSync, SyncReport, Vault};
use ratatui::prelude::*;

use crate::app::{App, PickEntry, PickerMode};
use crate::tree::Tree;

/// How often the loop polls the relay for remote changes.
const SERVER_POLL: Duration = Duration::from_secs(10);

/// The live backend: the vault plus its folder + relay sync wiring. Absent if
/// the canonical store couldn't be opened (the UI still browses files directly).
struct Backend {
    vault: Vault,
    folder: Option<PathBuf>,
    key: Option<[u8; KEY_BYTES]>,
    dirty: Arc<AtomicBool>,
    server: Option<ServerSync>,
    rt: Option<tokio::runtime::Runtime>,
    last_server_poll: Instant,
}

impl Backend {
    /// Sync via the shared folder (watcher-triggered or on demand).
    fn sync_folder_now(&mut self) -> SyncReport {
        match self.folder.clone() {
            Some(folder) => self.vault.sync_folder(&folder, self.key.as_ref()).unwrap_or_default(),
            None => SyncReport::default(),
        }
    }

    /// Sync via the relay (timer-triggered or on demand).
    fn sync_server_now(&mut self) -> SyncReport {
        self.last_server_poll = Instant::now();
        let (Some(rt), Some(server)) = (self.rt.as_ref(), self.server.as_mut()) else {
            return SyncReport::default();
        };
        rt.block_on(server.sync(&mut self.vault)).unwrap_or_default()
    }

    /// Both transports — used after local edits so changes propagate at once.
    fn sync(&mut self) -> SyncReport {
        let mut r = self.sync_folder_now();
        r.merge(self.sync_server_now());
        r
    }
}

fn main() -> Result<()> {
    // One-shot subcommands for moving the vault key between devices.
    let argv: Vec<String> = std::env::args().collect();
    match argv.get(1).map(String::as_str) {
        Some("share") => return cmd_share(),
        Some("import") => return cmd_import(argv.get(2).map(String::as_str)),
        _ => {}
    }

    let cfg = Config::load();
    let root = vault_dir(&cfg);
    if !root.is_dir() {
        eprintln!("notakase: vault directory not found: {}", root.display());
        std::process::exit(1);
    }

    // Open the vault through core: ingest on-disk edits into the canonical
    // per-note CRDT documents and materialize notes back to plain files.
    let vault = match Vault::open(&root, config::data_dir()) {
        Ok(v) => Some(v),
        Err(e) => {
            eprintln!("notakase: backend unavailable, browsing files directly ({e})");
            None
        }
    };

    // Resolve sync config: folder, relay, and (if encrypting) the vault key.
    let folder = cfg.sync_folder_path();
    let server_url = cfg.server_base_url.trim().to_string();
    let manifest_id = cfg.server_manifest_id.trim().to_string();
    let server_configured = !server_url.is_empty() && !manifest_id.is_empty();

    let mut key = None;
    if (folder.is_some() || server_configured) && cfg.encrypt {
        match vaultkey::load_or_create(&LibSecretKeyStore::new()) {
            Ok(k) => key = Some(k),
            Err(e) => eprintln!("notakase: encryption unavailable ({e})"),
        }
    }
    let encrypted = key.is_some();

    // Build the relay client + its runtime (both or neither).
    let mut server = if server_configured {
        match ServerSync::new(&server_url, &manifest_id, key) {
            Ok(s) => Some(s),
            Err(e) => {
                eprintln!("notakase: relay unavailable ({e})");
                None
            }
        }
    } else {
        None
    };
    let rt = if server.is_some() {
        tokio::runtime::Builder::new_current_thread().enable_all().build().ok()
    } else {
        None
    };
    if rt.is_none() {
        server = None;
    }

    // Watch the sync folder — the watcher only flips a flag; the loop syncs.
    let dirty = Arc::new(AtomicBool::new(false));
    let _watcher = match (&folder, &vault) {
        (Some(f), Some(_)) => watch::spawn(f, dirty.clone()),
        _ => None,
    };

    let mut backend = vault.map(|v| Backend {
        vault: v,
        folder: folder.clone(),
        key,
        dirty: dirty.clone(),
        server,
        rt,
        last_server_poll: Instant::now(),
    });

    // Initial sync before the first paint, so the tree already shows notes
    // pulled from other devices.
    let initial = backend.as_mut().map(Backend::sync);

    let tree = Tree::build(&root);
    let mut app = App::new(tree, root);
    app.sync_folder = folder
        .as_ref()
        .and_then(|f| f.file_name().map(|s| s.to_string_lossy().into_owned()));
    app.server = server_configured.then(|| short_host(&server_url));
    app.encrypted = encrypted;
    if let Some(rep) = &initial {
        record_sync(&mut app, rep);
    }

    let mut terminal = setup_terminal()?;
    install_panic_hook();
    let res = run(&mut terminal, &mut app, &mut backend);
    restore_terminal(&mut terminal)?;
    res
}

/// `notakase share` — print a share link carrying the vault key.
fn cmd_share() -> Result<()> {
    let key = vaultkey::load_or_create(&LibSecretKeyStore::new())?;
    println!("{}", sharelink::encode("vault", &key));
    eprintln!("(share this only over a trusted channel — it contains your key)");
    Ok(())
}

/// `notakase import <link>` — store a vault key received from another device.
fn cmd_import(link: Option<&str>) -> Result<()> {
    let Some(link) = link else {
        eprintln!("usage: notakase import notakase://share/vault#k=…");
        std::process::exit(2);
    };
    let payload = sharelink::decode(link).map_err(|e| anyhow::anyhow!("bad share link: {e}"))?;
    vaultkey::set(&LibSecretKeyStore::new(), &payload.key)?;
    println!("vault key imported — encrypted sync will now decrypt on this device");
    Ok(())
}

/// A compact host label for the status bar (strips scheme and path).
fn short_host(url: &str) -> String {
    let s = url.split("://").nth(1).unwrap_or(url);
    s.split('/').next().unwrap_or(s).to_string()
}

/// Vault directory precedence: CLI arg → $NOTAKASE_NOTES → config → bundled
/// sample vault (crate lives at crates/notakase-tui).
fn vault_dir(cfg: &Config) -> PathBuf {
    if let Some(arg) = std::env::args().nth(1) {
        return PathBuf::from(arg);
    }
    if let Some(env) = std::env::var_os("NOTAKASE_NOTES") {
        return PathBuf::from(env);
    }
    if !cfg.vault_dir.trim().is_empty() {
        return cfg.resolved_vault_dir();
    }
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../notes"))
}

fn sync_label(r: &SyncReport) -> String {
    if r.pulled == 0 && r.pushed == 0 {
        "up to date".to_string()
    } else {
        format!("{}↓ {}↑", r.pulled, r.pushed)
    }
}

fn run(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    app: &mut App,
    backend: &mut Option<Backend>,
) -> Result<()> {
    while !app.quit {
        // Pull remote edits and reload if anything changed: the shared folder
        // when the watcher flags a change, the relay on a timer.
        if let Some(b) = backend.as_mut() {
            let mut rep: Option<SyncReport> = None;
            if b.dirty.swap(false, Ordering::Relaxed) {
                rep.get_or_insert_with(SyncReport::default).merge(b.sync_folder_now());
            }
            if b.server.is_some() && b.last_server_poll.elapsed() >= SERVER_POLL {
                rep.get_or_insert_with(SyncReport::default).merge(b.sync_server_now());
            }
            if let Some(rep) = rep {
                if rep.changed {
                    app.reload();
                }
                record_sync(app, &rep);
            }
        }

        app.refresh_preview();
        terminal.draw(|f| ui::render(f, app))?;

        if !event::poll(Duration::from_millis(250))? {
            continue;
        }
        if let Event::Key(key) = event::read()? {
            if key.kind == KeyEventKind::Release {
                continue;
            }
            // A prompt (new/rename/delete) captures all input until it resolves.
            if app.prompt.is_some() {
                handle_prompt_key(app, backend, key.code);
                continue;
            }
            // The find/search/command overlay likewise captures input.
            if app.picker.is_some() {
                handle_picker_key(app, backend, key);
                continue;
            }
            let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
            app.notice = None;
            match key.code {
                KeyCode::Char('q') | KeyCode::Esc => app.quit = true,
                KeyCode::Char('c') if ctrl => app.quit = true,

                KeyCode::Char('j') | KeyCode::Down => app.move_cursor(1),
                KeyCode::Char('k') | KeyCode::Up => app.move_cursor(-1),
                KeyCode::Char('g') => app.go_top(),
                KeyCode::Char('G') => app.go_bottom(),

                KeyCode::Char('h') | KeyCode::Left => app.collapse(),
                KeyCode::Char(' ') => app.toggle(),
                // Enter / l: open a folder, or edit a note in $EDITOR
                KeyCode::Char('l') | KeyCode::Right => app.expand(),
                KeyCode::Enter => {
                    if app.selected_file().is_some() {
                        edit_selected(terminal, app, backend)?;
                    } else {
                        app.expand();
                    }
                }
                KeyCode::Char('e') => edit_selected(terminal, app, backend)?,

                // create (seed + open editor) / rename / delete
                KeyCode::Char('a') | KeyCode::Char('n') => new_note(terminal, app, backend)?,
                KeyCode::Char('r') => app.begin_rename(),
                KeyCode::Char('d') => app.begin_delete(),

                // fuzzy open (Ctrl-p) / full-text search (/) / command palette (:)
                KeyCode::Char('p') if ctrl => open_picker(app, backend, PickerMode::Files),
                KeyCode::Char('/') => open_picker(app, backend, PickerMode::Search),
                KeyCode::Char(':') => app.open_command_palette(),

                KeyCode::Char('J') => app.scroll_preview(1),
                KeyCode::Char('K') => app.scroll_preview(-1),
                KeyCode::Char('f') if ctrl => app.scroll_preview(10),
                KeyCode::Char('b') if ctrl => app.scroll_preview(-10),

                KeyCode::Tab => app.toggle_tree(),
                _ => {}
            }
        }
    }
    Ok(())
}

/// Route a keystroke into the active prompt; on Enter, run the mutation.
fn handle_prompt_key(app: &mut App, backend: &mut Option<Backend>, code: KeyCode) {
    use crate::app::PromptKind;
    match code {
        KeyCode::Esc => app.prompt = None,
        KeyCode::Backspace => app.prompt_backspace(),
        KeyCode::Enter => {
            let Some(p) = app.prompt.take() else { return };
            match p.kind {
                PromptKind::Rename { old } => finish_rename(app, backend, &old, &p.buf),
                PromptKind::ConfirmDelete { path } => finish_delete(app, backend, &path),
            }
        }
        KeyCode::Char(c) => {
            // delete confirm: y = do it, anything else cancels
            if let Some(p) = app.prompt.as_ref() {
                if let PromptKind::ConfirmDelete { path } = &p.kind {
                    let path = path.clone();
                    app.prompt = None;
                    if c == 'y' || c == 'Y' {
                        finish_delete(app, backend, &path);
                    }
                    return;
                }
            }
            app.prompt_push(c);
        }
        _ => {}
    }
}

/// Collect the searchable note set: from the in-memory vault when available,
/// else by reading the tree's files off disk.
fn collect_entries(app: &App, backend: &Option<Backend>) -> Vec<PickEntry> {
    if let Some(b) = backend {
        return b
            .vault
            .live_notes()
            .into_iter()
            .map(|n| PickEntry::note(n.doc.path(), n.doc.body()))
            .collect();
    }
    app.tree
        .nodes
        .iter()
        .filter(|n| !n.is_dir)
        .map(|n| {
            let rel = n
                .path
                .strip_prefix(&app.root)
                .unwrap_or(&n.path)
                .to_string_lossy()
                .replace('\\', "/");
            PickEntry::note(rel, std::fs::read_to_string(&n.path).unwrap_or_default())
        })
        .collect()
}

fn open_picker(app: &mut App, backend: &Option<Backend>, mode: PickerMode) {
    let entries = collect_entries(app, backend);
    app.open_picker(mode, entries);
}

/// Route a keystroke into the open find/search/command overlay.
fn handle_picker_key(app: &mut App, backend: &mut Option<Backend>, key: crossterm::event::KeyEvent) {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    match key.code {
        KeyCode::Esc => app.close_picker(),
        KeyCode::Enter => {
            // a command runs its action; a note jumps to it in the tree
            if let Some(cmd) = app.picker_selected_command() {
                app.close_picker();
                dispatch_command(app, backend, cmd);
            } else if let Some(rel) = app.picker_selected_rel() {
                app.close_picker();
                app.show_tree = true;
                app.select_path(&rel);
            } else {
                app.close_picker();
            }
        }
        KeyCode::Backspace => app.picker_backspace(),
        KeyCode::Up => app.picker_move(-1),
        KeyCode::Down => app.picker_move(1),
        KeyCode::Char('p') if ctrl => app.picker_move(-1),
        KeyCode::Char('n') if ctrl => app.picker_move(1),
        KeyCode::Char(c) if !ctrl => app.picker_input(c),
        _ => {}
    }
}

/// Run a command-palette action.
fn dispatch_command(app: &mut App, backend: &mut Option<Backend>, cmd: crate::app::CommandId) {
    match cmd {
        crate::app::CommandId::SyncNow => {
            if app.sync_folder.is_none() && app.server.is_none() {
                app.notice = Some("sync not configured (local only)".into());
                return;
            }
            match backend.as_mut() {
                Some(b) => {
                    let rep = b.sync();
                    if rep.changed {
                        app.reload();
                    }
                    record_sync(app, &rep);
                    app.notice = Some(format!("synced · {}↓ {}↑", rep.pulled, rep.pushed));
                }
                None => app.notice = Some("backend unavailable".into()),
            }
        }
    }
}

/// Record a completed sync: refresh the last-sync time (only when a transport
/// is actually configured) and the short result summary.
fn record_sync(app: &mut App, rep: &SyncReport) {
    if app.sync_folder.is_some() || app.server.is_some() {
        app.sync_msg = Some(sync_label(rep));
        app.last_sync_at = Some(chrono::Local::now().format("%H:%M").to_string());
    }
}

/// Ensure a note path ends in `.md`.
fn ensure_md(p: &str) -> String {
    let p = p.trim();
    if p.ends_with(".md") {
        p.to_string()
    } else {
        format!("{p}.md")
    }
}

fn finish_rename(app: &mut App, backend: &mut Option<Backend>, old: &str, raw: &str) {
    let new = ensure_md(raw);
    with_vault(app, backend, |v| v.rename_note(old, &new), |app| {
        app.reload();
        app.select_path(&new);
        app.notice = Some(format!("renamed to {new}"));
    });
}

fn finish_delete(app: &mut App, backend: &mut Option<Backend>, path: &str) {
    let path = path.to_string();
    with_vault(app, backend, |v| v.delete_note(&path), |app| {
        app.reload();
        app.notice = Some(format!("deleted {path}"));
    });
}

/// Run a fallible vault mutation, then push to sync and run `on_ok`. Surfaces
/// any error as a status-bar notice instead of crashing.
fn with_vault<M, K>(app: &mut App, backend: &mut Option<Backend>, mutate: M, on_ok: K)
where
    M: FnOnce(&mut Vault) -> anyhow::Result<()>,
    K: FnOnce(&mut App),
{
    let Some(b) = backend.as_mut() else {
        app.notice = Some("backend unavailable".into());
        return;
    };
    match mutate(&mut b.vault) {
        Ok(()) => {
            let _ = b.sync(); // propagate the change immediately if syncing
            on_ok(app);
        }
        Err(e) => app.notice = Some(e.to_string()),
    }
}

/// Create a new note seeded with a title + date/time, open it in $EDITOR so the
/// title can be replaced right away, and reveal it in the tree on return.
fn new_note(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    app: &mut App,
    backend: &mut Option<Backend>,
) -> Result<()> {
    let Some(b) = backend.as_mut() else {
        app.notice = Some("backend unavailable".into());
        return Ok(());
    };
    let prefix = app.current_folder_prefix();
    let rel = format!("{prefix}{}", unique_new_name(&b.vault, &prefix));
    if let Err(e) = b.vault.create_note(&rel, &new_note_template()) {
        app.notice = Some(e.to_string());
        return Ok(());
    }
    let _ = b.sync();
    // show it immediately, then edit; it stays visible whether or not it's saved
    app.reload();
    app.select_path(&rel);

    let abs = app.root.join(&rel);
    edit_file(terminal, &abs)?;
    let final_rel = post_edit(app, backend, &rel);
    app.notice = Some(format!("created {final_rel}"));
    Ok(())
}

/// Suspend the TUI, edit the selected note in $EDITOR, then re-ingest + reload.
fn edit_selected(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    app: &mut App,
    backend: &mut Option<Backend>,
) -> Result<()> {
    let Some(idx) = app.selected() else {
        return Ok(());
    };
    if app.tree.nodes[idx].is_dir {
        return Ok(());
    }
    let rel = app.rel_of(idx);
    let path = app.tree.nodes[idx].path.clone();
    edit_file(terminal, &path)?;
    post_edit(app, backend, &rel);
    Ok(())
}

/// After an editor session on the note at `rel`: re-ingest the edit, sync the
/// filename to the title (Obsidian-style), push, reload, and keep it selected.
/// Returns the note's (possibly renamed) relative path.
fn post_edit(app: &mut App, backend: &mut Option<Backend>, rel: &str) -> String {
    let mut final_rel = rel.to_string();
    if let Some(b) = backend.as_mut() {
        let _ = b.vault.rescan();
        if let Ok(Some(new_rel)) = b.vault.retitle_from_body(rel) {
            final_rel = new_rel;
        }
        let _ = b.sync();
    }
    app.reload();
    app.select_path(&final_rel);
    final_rel
}

/// Leave the TUI, run $EDITOR (then $VISUAL, then vi) on `path`, and re-enter.
/// Editors open at line 1, so the cursor lands on the title.
fn edit_file(terminal: &mut Terminal<CrosstermBackend<Stdout>>, path: &std::path::Path) -> Result<()> {
    let editor = std::env::var("VISUAL")
        .or_else(|_| std::env::var("EDITOR"))
        .unwrap_or_else(|_| "vi".to_string());
    let mut parts = editor.split_whitespace();
    let Some(prog) = parts.next() else {
        return Ok(());
    };
    let args: Vec<&str> = parts.collect();

    restore_terminal(terminal)?;
    let _ = std::process::Command::new(prog).args(&args).arg(path).status();
    enable_raw_mode()?;
    execute!(io::stdout(), EnterAlternateScreen, EnableMouseCapture)?;
    terminal.clear()?;
    Ok(())
}

/// A fresh note's starting content: an editable title and a date/time subtitle.
fn new_note_template() -> String {
    let now = chrono::Local::now();
    format!("# New note\n\n{}\n\n", now.format("%A, %B %-d, %Y · %-I:%M %p"))
}

/// A non-colliding "New note.md" (then "New note 2.md", …) within `prefix`.
fn unique_new_name(vault: &Vault, prefix: &str) -> String {
    let mut i = 1;
    loop {
        let name = if i == 1 {
            "New note.md".to_string()
        } else {
            format!("New note {i}.md")
        };
        let taken = vault
            .note_by_path(&format!("{prefix}{name}"))
            .map(|n| !n.doc.deleted())
            .unwrap_or(false);
        if !taken {
            return name;
        }
        i += 1;
    }
}

fn setup_terminal() -> Result<Terminal<CrosstermBackend<Stdout>>> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    Ok(Terminal::new(backend)?)
}

fn restore_terminal(terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> Result<()> {
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen, DisableMouseCapture)?;
    terminal.show_cursor()?;
    Ok(())
}

/// Restore the terminal before unwinding so a panic doesn't leave the user in a
/// broken raw-mode alternate screen.
fn install_panic_hook() {
    let original = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen, DisableMouseCapture);
        original(info);
    }));
}
