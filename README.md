# notakase

A keyboard-first, terminal-native **notes** app for Omarchy — a sibling to
`todarchy`. Your notes are plain markdown files in plain folders, backed by a
per-note CRDT store for conflict-free sync across devices.

- **Left pane:** a drill-down tree of your note folders.
- **Right pane:** a themed, live markdown preview of the selected note.
- **Colors:** the accent is read straight from your *active* Omarchy theme
  (`~/.config/omarchy/current/theme/colors.toml`), and everything else uses the
  terminal's ANSI palette — so notakase always matches your desktop. Switch
  themes and it recolors instantly.

Built on the same stack as todarchy: Rust + `ratatui`, `pulldown-cmark`, ANSI
colors with one truecolor accent, Nerd Font glyphs with an ASCII fallback, and
the airy "hairlines, not boxes" aesthetic.

## Run

```bash
cargo run                      # browses the bundled sample vault in ./notes
cargo run -- ~/Documents/notes # point it at your own notes
```

## Keys

| Key             | Action                                   |
|-----------------|------------------------------------------|
| `j` / `k`       | Move selection down / up                 |
| `g` / `G`       | Jump to top / bottom                     |
| `l` / `→`       | Expand folder                            |
| `h`             | Collapse folder / go to parent           |
| `↵`             | Open folder, or edit note in `$EDITOR`   |
| `e`             | Edit selected note in `$EDITOR`          |
| `a` / `n`       | New note → opens `$EDITOR` on a seeded title + date |
| `r`             | Rename / move selected note              |
| `d`             | Delete selected note (confirms)          |
| `Ctrl-p`        | Fuzzy-find a note by path                |
| `/`             | Full-text search across note bodies      |
| `:`             | Command palette (e.g. Sync now)          |
| `J` / `K`       | Scroll the preview                       |
| `Ctrl-f` / `-b` | Page-scroll the preview                  |
| `Tab`           | Zen mode (hide the tree)                 |
| `q` / `Esc`     | Quit                                     |

Pressing `a` drops a new note into the folder you're standing in, seeded with a
`# New note` title and a date/time line, and opens your editor with the cursor
on the title. When you save, the **filename follows the title** — edit the `#`
heading to `# Groceries` and the file becomes `Groceries.md` (Obsidian-style;
the note keeps its identity, so history survives the rename). Rename (`r`) also
accepts slashes to move a note into (new) folders.

## Environment

| Variable          | Effect                                                    |
|-------------------|-----------------------------------------------------------|
| `NOTAKASE_NOTES`  | Notes directory (overridden by a CLI path argument)       |
| `NOTAKASE_ACCENT` | Force the accent (hex `#rrggbb`, color name, or 0–255)    |
| `NOTAKASE_ASCII`  | `1` → plain-Unicode glyphs instead of Nerd Font           |

## Layout

A cargo workspace, mirroring todarchy: a core data/sync crate and a TUI crate.

```
crates/
├── notakase-core/          data + sync (no UI)
│   ├── doc.rs              ★ per-note Automerge CRDT document
│   ├── store.rs           ★ Vault: notes ⇄ plain .md files (+ nesting)
│   ├── config.rs          ★ hand-edited TOML config
│   ├── sync.rs            ★ SyncStatus vocabulary
│   ├── cryptobox.rs        ChaCha20-Poly1305 sealed envelope  (from todarchy)
│   ├── keystore.rs         libsecret key storage              (from todarchy)
│   ├── sharelink.rs        notakase://share/<id>#k=… codec    (from todarchy)
│   └── server_client.rs    /doc/:id relay client              (from todarchy)
└── notakase-tui/           the terminal UI
    ├── main.rs             lifecycle + event loop + opens the Vault
    ├── app.rs · ui.rs      state + rendering
    ├── tree.rs             filesystem → collapsible folder tree
    ├── markdown.rs         pulldown-cmark → themed ratatui lines
    └── theme.rs            live Omarchy accent + glyph set
notes/                      default sample vault (plain markdown)
```

## How storage works

Notes are plain files *and* CRDT documents. On launch the TUI opens the vault
through `notakase-core`:

1. **Ingest** — edits made to the plain `.md` files fold into each note's
   canonical Automerge document (`~/.local/share/notakase/notes/<id>.automerge`).
2. **Materialize** — notes are written back out as plain files, creating any
   parent folders (this is where **arbitrarily deep nesting** lands).
3. **Persist** — the canonical documents are saved; these are what sync ships.

Each note's id is stable and independent of its path, and the full relative
path (`Projects/client/2026/q3/research/sources/paper-notes.md`) lives inside
the document — so moves, renames, and deep nesting all survive sync.

## Sync

Sync is configured by hand-editing `~/.config/notakase/config.toml` — no
in-app settings screen, true to todarchy's ethos.

```toml
# A directory of plain markdown files (your vault).
vault_dir = "~/Documents/notakase"

# A folder your OS keeps in sync across devices (Syncthing / Dropbox / iCloud).
# notakase mirrors each note's CRDT document into it and merges on change.
sync_folder = "~/Syncthing/notakase"

# A self-hosted relay (notakase-server), as an alternative or addition to a
# folder. The manifest id lists your note ids and must match on every device.
server_base_url = "https://notes.example.com"
server_manifest_id = "manifest_<your-shared-id>"

# Encrypt the synced copies (folder + relay) with ChaCha20-Poly1305. The key
# lives in your OS keyring; move it to another device with `notakase share`.
encrypt = true
```

Either transport (or both) can be on. notakase watches the folder and polls the
relay every few seconds: edits from another device are pulled, merged
(conflict-free, per note), and shown live. Because every note is its own CRDT,
two devices editing different notes never conflict, and concurrent edits to the
same note merge at the character level.

The status bar shows the sync target, a lock when encrypting, and the **time of
the last successful sync**. Sync also runs on demand: press `:` for the command
palette and choose **Sync now**.

### Pairing devices (encrypted sync)

The vault key is per-device. To share it, on the first device run:

```bash
notakase share            # prints notakase://share/vault#k=…
```

and on each other device:

```bash
notakase import 'notakase://share/vault#k=…'
```

Send the link over a trusted channel — it contains your key.

## Embedding a task list

A note can embed a **live task list** from the sibling task app (todokase /
todarchy) with a fenced `todokase` block:

````markdown
```todokase
project: notakase
status: open
```
````

Keys: `project` (name or id), `context` (`@work`), `status` (`open`/`done`/`all`,
default open), `limit`. The block stores only the *query* — notakase reads the
task app's `tasks.json` and renders the matching tasks fresh each time you open
the note. Other markdown viewers just show it as a code block, so notes stay
portable. Read-only for now (checking tasks off from a note is a future step).

## Roadmap

- **M1 (done)** — workspace, per-note CRDT store, vault ↔ files, deep nesting
  front-to-back, TUI loading through core.
- **M2 (done)** — folder sync (Syncthing/Dropbox/iCloud) + file watcher +
  ChaCha20-Poly1305 encryption of synced copies + a live sync indicator.
- **M4 (done)** — in-app create / rename / delete (rename keeps note identity)
  + `$EDITOR` hand-off that re-ingests on return + a confirm-to-delete prompt.
- **Find (done)** — `Ctrl-p` fuzzy-open by path and `/` full-text search across
  note bodies, in a single overlay picker that jumps to the chosen note.
- **Command palette (done)** — `:` opens a palette (first command: Sync now);
  the status bar shows the last-sync time.
- **Task embeds (done)** — a `todokase` fenced block renders a live, read-only
  task list from the sibling task app.
- **M3 (done)** — self-hosted relay sync (`/doc/:id` + ETag polling), a manifest
  CRDT listing note ids for discovery, and `notakase share`/`import` to move the
  vault key between devices.
