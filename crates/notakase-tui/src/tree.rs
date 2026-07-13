// tree.rs — the filesystem-backed folder tree shown in the left pane.
//
// Notes are just files: a directory of `.md` files and subdirectories. We read
// the whole tree eagerly into a flat arena (fast enough for any real notes
// vault) and expose a `visible()` walk that respects each folder's collapsed
// state. Directories sort first, then files, both case-insensitively.

use std::path::{Path, PathBuf};

pub struct Node {
    pub name: String,
    pub path: PathBuf,
    pub is_dir: bool,
    pub children: Vec<usize>,
    pub expanded: bool,
}

pub struct Tree {
    pub nodes: Vec<Node>,
    pub root: usize,
}

/// A visible row: an index into `nodes` plus its indentation depth.
#[derive(Clone, Copy)]
pub struct Row {
    pub node: usize,
    pub depth: u16,
}

impl Tree {
    pub fn build(root: &Path) -> Tree {
        let mut nodes = Vec::new();
        let name = root
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "notes".to_string());
        let root_idx = nodes.len();
        nodes.push(Node {
            name,
            path: root.to_path_buf(),
            is_dir: true,
            children: Vec::new(),
            expanded: true,
        });
        let kids = read_children(root, &mut nodes);
        nodes[root_idx].children = kids;
        Tree { nodes, root: root_idx }
    }

    /// Depth-first walk of the expanded tree, starting below the root (the root
    /// itself is implicit — its children render at depth 0).
    pub fn visible(&self) -> Vec<Row> {
        let mut rows = Vec::new();
        self.walk(self.root, 0, &mut rows, true);
        rows
    }

    fn walk(&self, idx: usize, depth: u16, rows: &mut Vec<Row>, is_root: bool) {
        let node = &self.nodes[idx];
        if !is_root {
            rows.push(Row { node: idx, depth });
        }
        if is_root || node.expanded {
            let child_depth = if is_root { 0 } else { depth + 1 };
            for &c in &node.children {
                self.walk(c, child_depth, rows, false);
            }
        }
    }

    pub fn toggle(&mut self, idx: usize) {
        let n = &mut self.nodes[idx];
        if n.is_dir {
            n.expanded = !n.expanded;
        }
    }
}

fn read_children(dir: &Path, nodes: &mut Vec<Node>) -> Vec<usize> {
    let mut entries: Vec<(bool, String, PathBuf)> = Vec::new();
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    for entry in rd.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue; // skip hidden
        }
        let is_dir = path.is_dir();
        if !is_dir && path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue; // only markdown files
        }
        entries.push((is_dir, name, path));
    }
    // directories first, then files; each group case-insensitive alphabetical
    entries.sort_by(|a, b| {
        b.0.cmp(&a.0)
            .then_with(|| a.1.to_lowercase().cmp(&b.1.to_lowercase()))
    });

    let mut out = Vec::with_capacity(entries.len());
    for (is_dir, name, path) in entries {
        let idx = nodes.len();
        nodes.push(Node {
            name,
            path: path.clone(),
            is_dir,
            children: Vec::new(),
            expanded: false,
        });
        out.push(idx);
        if is_dir {
            let kids = read_children(&path, nodes);
            nodes[idx].children = kids;
        }
    }
    out
}
