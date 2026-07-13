// app.rs — all mutable state + the input handling. Rendering lives in ui.rs and
// never mutates; it reads this.

use std::cell::Cell;
use std::collections::HashSet;
use std::path::PathBuf;

use ratatui::style::Color;
use ratatui::text::Line;

use crate::markdown;
use crate::theme;
use crate::tree::{Row, Tree};

/// A one-line input shown in the status bar for create/rename/delete.
pub enum PromptKind {
    Rename { old: String },
    ConfirmDelete { path: String },
}

impl PromptKind {
    pub fn label(&self) -> &'static str {
        match self {
            PromptKind::Rename { .. } => "rename",
            PromptKind::ConfirmDelete { .. } => "delete",
        }
    }
}

pub struct Prompt {
    pub kind: PromptKind,
    pub buf: String,
    pub err: Option<String>,
}

/// The overlay picker: fuzzy file-open, or full-text search.
#[derive(PartialEq, Clone, Copy)]
pub enum PickerMode {
    Files,
    Search,
}

/// One searchable note: its vault-relative path and its body.
pub struct PickEntry {
    pub rel: String,
    pub body: String,
}

/// A filtered result: an index into `entries` plus an optional match snippet.
pub struct PickResult {
    pub idx: usize,
    pub snippet: Option<String>,
}

pub struct Picker {
    pub mode: PickerMode,
    pub query: String,
    pub entries: Vec<PickEntry>,
    pub results: Vec<PickResult>,
    pub sel: usize,
}

impl Picker {
    pub fn title(&self) -> &'static str {
        match self.mode {
            PickerMode::Files => "open",
            PickerMode::Search => "search",
        }
    }

    /// Recompute results for the current query and mode.
    pub fn recompute(&mut self) {
        self.results.clear();
        match self.mode {
            PickerMode::Files => {
                let mut scored: Vec<(i64, usize)> = self
                    .entries
                    .iter()
                    .enumerate()
                    .filter_map(|(i, e)| fuzzy_cost(&self.query, &e.rel).map(|c| (c, i)))
                    .collect();
                scored.sort_by(|a, b| {
                    a.0.cmp(&b.0)
                        .then(self.entries[a.1].rel.len().cmp(&self.entries[b.1].rel.len()))
                });
                self.results = scored
                    .into_iter()
                    .take(200)
                    .map(|(_, idx)| PickResult { idx, snippet: None })
                    .collect();
            }
            PickerMode::Search => {
                let q = self.query.trim().to_lowercase();
                if !q.is_empty() {
                    for (idx, e) in self.entries.iter().enumerate() {
                        if let Some(snip) = first_match_line(&e.body, &q) {
                            self.results.push(PickResult { idx, snippet: Some(snip) });
                        } else if e.rel.to_lowercase().contains(&q) {
                            self.results.push(PickResult { idx, snippet: None });
                        }
                    }
                    self.results.truncate(200);
                }
            }
        }
        self.sel = self.sel.min(self.results.len().saturating_sub(1));
    }
}

/// Fuzzy subsequence match cost (lower is better); `None` if `query` is not a
/// subsequence of `target`. Empty query matches everything.
fn fuzzy_cost(query: &str, target: &str) -> Option<i64> {
    if query.is_empty() {
        return Some(0);
    }
    let q: Vec<char> = query.to_lowercase().chars().collect();
    let t: Vec<char> = target.to_lowercase().chars().collect();
    let (mut qi, mut first, mut last, mut gaps, mut prev) = (0usize, None, 0usize, 0i64, None);
    for (i, c) in t.iter().enumerate() {
        if qi < q.len() && *c == q[qi] {
            if first.is_none() {
                first = Some(i);
            }
            if let Some(p) = prev {
                if i != p + 1 {
                    gaps += 1;
                }
            }
            prev = Some(i);
            last = i;
            qi += 1;
        }
    }
    if qi != q.len() {
        return None;
    }
    let first = first.unwrap_or(0) as i64;
    Some(first + (last as i64 - first) + gaps * 2)
}

/// The first body line containing `needle` (already lowercased), trimmed and
/// length-capped for display.
fn first_match_line(body: &str, needle: &str) -> Option<String> {
    for line in body.lines() {
        if line.to_lowercase().contains(needle) {
            let t = line.trim();
            let snip: String = t.chars().take(80).collect();
            return Some(snip);
        }
    }
    None
}

pub struct App {
    pub tree: Tree,
    pub rows: Vec<Row>,
    pub cursor: usize,
    pub accent: Color,
    pub root: PathBuf,

    pub scroll: u16,
    pub max_scroll: Cell<u16>,

    pub show_tree: bool,
    pub quit: bool,

    // Sync status (for the status bar). Set by main after resolving config.
    pub sync_folder: Option<String>,
    pub server: Option<String>,
    pub encrypted: bool,
    pub sync_msg: Option<String>,

    // In-app editing (M4): an active prompt and a transient notice line.
    pub prompt: Option<Prompt>,
    pub notice: Option<String>,

    // Fuzzy-find / search overlay.
    pub picker: Option<Picker>,

    // Precomputed preview body for the selected node (rebuilt only when the
    // selection changes), so ui::render can stay a pure function of &App.
    pub preview: Vec<Line<'static>>,
    /// For files: (line count, word count). None for folders.
    pub preview_stats: Option<(usize, usize)>,
    preview_for: Option<usize>,
}

impl App {
    pub fn new(tree: Tree, root: PathBuf) -> App {
        let rows = tree.visible();
        let mut app = App {
            tree,
            rows,
            cursor: 0,
            accent: theme::accent(),
            root,
            scroll: 0,
            max_scroll: Cell::new(0),
            show_tree: true,
            quit: false,
            sync_folder: None,
            server: None,
            encrypted: false,
            sync_msg: None,
            prompt: None,
            notice: None,
            picker: None,
            preview: Vec::new(),
            preview_stats: None,
            preview_for: None,
        };
        app.refresh_preview();
        app
    }

    /// Rebuild the tree from disk after a sync brought in changes, preserving
    /// which folders are open and which note is selected (by path).
    pub fn reload(&mut self) {
        let expanded: HashSet<PathBuf> = self
            .tree
            .nodes
            .iter()
            .filter(|n| n.is_dir && n.expanded)
            .map(|n| n.path.clone())
            .collect();
        let selected_path = self.selected().map(|i| self.tree.nodes[i].path.clone());

        let mut tree = Tree::build(&self.root);
        for n in &mut tree.nodes {
            if n.is_dir && expanded.contains(&n.path) {
                n.expanded = true;
            }
        }
        self.tree = tree;
        self.rows = self.tree.visible();

        if let Some(sp) = selected_path {
            if let Some(pos) = self
                .rows
                .iter()
                .position(|r| self.tree.nodes[r.node].path == sp)
            {
                self.cursor = pos;
            }
        }
        self.cursor = self.cursor.min(self.rows.len().saturating_sub(1));
        self.preview_for = None; // force preview rebuild
        self.refresh_preview();
    }

    fn recompute(&mut self) {
        self.rows = self.tree.visible();
        if self.cursor >= self.rows.len() {
            self.cursor = self.rows.len().saturating_sub(1);
        }
    }

    /// The node index currently under the cursor, if any.
    pub fn selected(&self) -> Option<usize> {
        self.rows.get(self.cursor).map(|r| r.node)
    }

    /// Rebuild the preview body for the selected node if the selection changed.
    /// Call once per loop tick before rendering.
    pub fn refresh_preview(&mut self) {
        let idx = match self.selected() {
            Some(i) => i,
            None => {
                self.preview = Vec::new();
                self.preview_stats = None;
                self.preview_for = None;
                return;
            }
        };
        if self.preview_for == Some(idx) {
            return;
        }
        self.preview_for = Some(idx);

        let node = &self.tree.nodes[idx];
        if node.is_dir {
            self.preview_stats = None;
            self.preview = self.dir_summary(idx);
        } else {
            let text = std::fs::read_to_string(&node.path).unwrap_or_default();
            let lines = text.lines().count();
            let words = text.split_whitespace().count();
            self.preview_stats = Some((lines, words));
            self.preview = markdown::render(&text, self.accent);
        }
    }

    /// A quiet listing shown when a folder (not a file) is selected.
    fn dir_summary(&self, idx: usize) -> Vec<Line<'static>> {
        use ratatui::style::{Modifier, Style};
        use ratatui::text::Span;
        let g = theme::glyphs();
        let node = &self.tree.nodes[idx];
        let dirs = node
            .children
            .iter()
            .filter(|&&c| self.tree.nodes[c].is_dir)
            .count();
        let files = node.children.len() - dirs;

        let mut lines = vec![Line::from(Span::styled(
            format!("{dirs} folders · {files} notes"),
            Style::default().fg(Color::DarkGray),
        ))];
        lines.push(Line::from(""));
        for &c in &node.children {
            let child = &self.tree.nodes[c];
            let (icon, style) = if child.is_dir {
                (g.folder_closed, Style::default().add_modifier(Modifier::BOLD))
            } else {
                (g.note, Style::default())
            };
            lines.push(Line::from(vec![
                Span::styled(format!("{icon}  "), Style::default().fg(self.accent)),
                Span::styled(child.name.clone(), style),
            ]));
        }
        lines
    }

    // ---- movement ----

    pub fn move_cursor(&mut self, delta: i32) {
        if self.rows.is_empty() {
            return;
        }
        let n = self.rows.len() as i32;
        let c = (self.cursor as i32 + delta).clamp(0, n - 1);
        if c as usize != self.cursor {
            self.cursor = c as usize;
            self.scroll = 0;
        }
    }

    pub fn go_top(&mut self) {
        self.cursor = 0;
        self.scroll = 0;
    }

    pub fn go_bottom(&mut self) {
        self.cursor = self.rows.len().saturating_sub(1);
        self.scroll = 0;
    }

    /// `l` / `Enter` — expand a folder (or step onto its first child if already
    /// open). No-op on a file.
    pub fn expand(&mut self) {
        let Some(idx) = self.selected() else { return };
        let node = &self.tree.nodes[idx];
        if !node.is_dir {
            return;
        }
        if node.expanded {
            // already open: move onto the first child
            if !node.children.is_empty() {
                self.move_cursor(1);
            }
        } else {
            self.tree.toggle(idx);
            self.recompute();
        }
    }

    /// `h` — collapse an open folder, else jump to the parent row.
    pub fn collapse(&mut self) {
        let Some(idx) = self.selected() else { return };
        let node = &self.tree.nodes[idx];
        if node.is_dir && node.expanded {
            self.tree.toggle(idx);
            self.recompute();
            return;
        }
        // jump to the nearest shallower row above
        let depth = self.rows[self.cursor].depth;
        if depth == 0 {
            return;
        }
        for i in (0..self.cursor).rev() {
            if self.rows[i].depth < depth {
                self.cursor = i;
                self.scroll = 0;
                break;
            }
        }
    }

    /// `Space` / `Enter` on a folder toggles it.
    pub fn toggle(&mut self) {
        let Some(idx) = self.selected() else { return };
        if self.tree.nodes[idx].is_dir {
            self.tree.toggle(idx);
            self.recompute();
        }
    }

    // ---- preview scroll ----

    pub fn scroll_preview(&mut self, delta: i32) {
        let max = self.max_scroll.get() as i32;
        let s = (self.scroll as i32 + delta).clamp(0, max);
        self.scroll = s as u16;
    }

    pub fn toggle_tree(&mut self) {
        self.show_tree = !self.show_tree;
    }

    // ---- in-app editing (M4) ----

    /// The selected node's path relative to the vault root, `/`-joined.
    pub fn rel_of(&self, idx: usize) -> String {
        let p = &self.tree.nodes[idx].path;
        p.strip_prefix(&self.root)
            .unwrap_or(p)
            .to_string_lossy()
            .replace('\\', "/")
    }

    /// Absolute path of the selected note, if a file is selected.
    pub fn selected_file(&self) -> Option<PathBuf> {
        let idx = self.selected()?;
        let node = &self.tree.nodes[idx];
        (!node.is_dir).then(|| node.path.clone())
    }

    /// The folder you're "standing in" as a `/`-terminated prefix (or "" for
    /// the vault root): the selected folder, or the parent of the selected note.
    pub fn current_folder_prefix(&self) -> String {
        self.selected()
            .map(|i| {
                let node = &self.tree.nodes[i];
                let rel = self.rel_of(i);
                if node.is_dir {
                    if rel.is_empty() {
                        String::new()
                    } else {
                        format!("{rel}/")
                    }
                } else {
                    match rel.rsplit_once('/') {
                        Some((dir, _)) => format!("{dir}/"),
                        None => String::new(),
                    }
                }
            })
            .unwrap_or_default()
    }

    /// Begin a rename prompt for the selected note (files only for now).
    pub fn begin_rename(&mut self) {
        let Some(i) = self.selected() else { return };
        if self.tree.nodes[i].is_dir {
            self.notice = Some("rename is for notes, not folders (yet)".into());
            return;
        }
        let rel = self.rel_of(i);
        self.prompt = Some(Prompt {
            kind: PromptKind::Rename { old: rel.clone() },
            buf: rel,
            err: None,
        });
    }

    /// Begin a delete confirmation for the selected note.
    pub fn begin_delete(&mut self) {
        let Some(i) = self.selected() else { return };
        if self.tree.nodes[i].is_dir {
            self.notice = Some("delete is for notes, not folders (yet)".into());
            return;
        }
        let rel = self.rel_of(i);
        self.prompt = Some(Prompt {
            kind: PromptKind::ConfirmDelete { path: rel },
            buf: String::new(),
            err: None,
        });
    }

    pub fn prompt_push(&mut self, c: char) {
        if let Some(p) = self.prompt.as_mut() {
            p.buf.push(c);
            p.err = None;
        }
    }

    pub fn prompt_backspace(&mut self) {
        if let Some(p) = self.prompt.as_mut() {
            p.buf.pop();
            p.err = None;
        }
    }

    /// Expand the ancestors of `rel` and move the cursor onto it.
    pub fn select_path(&mut self, rel: &str) {
        let target = self.root.join(rel);
        for n in &mut self.tree.nodes {
            if n.is_dir && target.starts_with(&n.path) {
                n.expanded = true;
            }
        }
        self.rows = self.tree.visible();
        if let Some(pos) = self
            .rows
            .iter()
            .position(|r| self.tree.nodes[r.node].path == target)
        {
            self.cursor = pos;
            self.scroll = 0;
        }
        self.cursor = self.cursor.min(self.rows.len().saturating_sub(1));
        self.preview_for_reset();
    }

    fn preview_for_reset(&mut self) {
        self.preview_for = None;
        self.refresh_preview();
    }

    // ---- fuzzy-find / search overlay ----

    pub fn open_picker(&mut self, mode: PickerMode, entries: Vec<PickEntry>) {
        let mut p = Picker { mode, query: String::new(), entries, results: Vec::new(), sel: 0 };
        p.recompute();
        self.picker = Some(p);
    }

    pub fn close_picker(&mut self) {
        self.picker = None;
    }

    pub fn picker_input(&mut self, c: char) {
        if let Some(p) = self.picker.as_mut() {
            p.query.push(c);
            p.sel = 0;
            p.recompute();
        }
    }

    pub fn picker_backspace(&mut self) {
        if let Some(p) = self.picker.as_mut() {
            p.query.pop();
            p.sel = 0;
            p.recompute();
        }
    }

    pub fn picker_move(&mut self, delta: i32) {
        if let Some(p) = self.picker.as_mut() {
            if p.results.is_empty() {
                return;
            }
            let n = p.results.len() as i32;
            p.sel = (p.sel as i32 + delta).rem_euclid(n) as usize;
        }
    }

    /// The vault-relative path of the highlighted result, if any.
    pub fn picker_selected_rel(&self) -> Option<String> {
        let p = self.picker.as_ref()?;
        let r = p.results.get(p.sel)?;
        Some(p.entries[r.idx].rel.clone())
    }
}

#[cfg(test)]
mod picker_tests {
    use super::*;

    fn picker(mode: PickerMode, query: &str, entries: Vec<(&str, &str)>) -> Picker {
        let mut p = Picker {
            mode,
            query: query.to_string(),
            entries: entries
                .into_iter()
                .map(|(rel, body)| PickEntry { rel: rel.into(), body: body.into() })
                .collect(),
            results: Vec::new(),
            sel: 0,
        };
        p.recompute();
        p
    }

    #[test]
    fn fuzzy_matches_subsequence_and_ranks_contiguous_higher() {
        assert!(fuzzy_cost("spec", "Projects/notakase/spec.md").is_some());
        assert!(fuzzy_cost("zzz", "spec.md").is_none());
        assert_eq!(fuzzy_cost("", "anything"), Some(0));
        let contiguous = fuzzy_cost("spec", "spec.md").unwrap();
        let scattered = fuzzy_cost("spec", "s.p.e.c.md").unwrap();
        assert!(contiguous < scattered, "contiguous should cost less");
    }

    #[test]
    fn files_mode_empty_query_lists_all() {
        let p = picker(PickerMode::Files, "", vec![("a.md", ""), ("b.md", "")]);
        assert_eq!(p.results.len(), 2);
    }

    #[test]
    fn files_mode_filters_by_fuzzy() {
        let p = picker(PickerMode::Files, "spec", vec![("Proj/spec.md", ""), ("recipes/soup.md", "")]);
        assert_eq!(p.results.len(), 1);
        assert_eq!(p.entries[p.results[0].idx].rel, "Proj/spec.md");
    }

    #[test]
    fn search_mode_matches_body_lines_with_snippet() {
        let p = picker(
            PickerMode::Search,
            "starter",
            vec![("a.md", "# T\n\nfeed the starter tonight"), ("b.md", "nothing here")],
        );
        assert_eq!(p.results.len(), 1);
        assert_eq!(p.entries[p.results[0].idx].rel, "a.md");
        assert!(p.results[0].snippet.as_deref().unwrap().contains("starter"));
    }

    #[test]
    fn search_mode_empty_query_shows_nothing() {
        let p = picker(PickerMode::Search, "  ", vec![("a.md", "x")]);
        assert!(p.results.is_empty());
    }
}
