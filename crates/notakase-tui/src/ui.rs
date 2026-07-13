// ui.rs — all rendering. Pure function of &App → frame; never mutates state.
// Aesthetic (borrowed from todarchy): minimal / airy — no panel boxes, just
// whitespace, dim hairline rules between columns, and an accent bar on the
// selected row. Colors are ANSI/indexed so the terminal's Omarchy theme paints
// the whole UI; the one accent hue comes from the live theme (theme::accent()).

use ratatui::{
    prelude::*,
    widgets::{
        Block, BorderType, Borders, Clear, List, ListItem, ListState, Padding, Paragraph,
        Scrollbar, ScrollbarOrientation, ScrollbarState, Wrap,
    },
};

use crate::app::{App, PickerMode};
use crate::theme::{accent, glyphs};

const DIM: Color = Color::DarkGray;

pub fn render(frame: &mut Frame, app: &App) {
    let area = frame.area();
    if area.width < 8 || area.height < 3 {
        return;
    }

    // vertical: 1 blank top pad · body · 1-line status bar
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(1), Constraint::Length(1)])
        .split(area);
    let body = rows[1];

    let show_tree = app.show_tree && body.width >= 40;
    let cols = if show_tree {
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(28), Constraint::Min(24)])
            .split(body)
    } else {
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Min(24)])
            .split(body)
    };

    if show_tree {
        render_tree(frame, app, cols[0]);
        render_preview(frame, app, cols[1]);
    } else {
        render_preview(frame, app, cols[0]);
    }
    render_status_bar(frame, app, rows[2]);

    if app.picker.is_some() {
        render_picker(frame, app, area);
    }
}

fn render_picker(frame: &mut Frame, app: &App, area: Rect) {
    let Some(p) = &app.picker else { return };
    let g = glyphs();
    let ac = accent();

    let width = area.width.min(84).max(40);
    let height = area.height.min(20).max(6);
    let rect = Rect {
        x: (area.width.saturating_sub(width)) / 2,
        y: area.height / 8,
        width,
        height,
    };
    frame.render_widget(Clear, rect);
    let icon = if p.mode == PickerMode::Search { g.search } else { g.folder_open };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(ac))
        .padding(Padding::horizontal(1))
        .title(Span::styled(format!(" {icon} {} ", p.title()), Style::default().fg(ac)));
    let inner = block.inner(rect);
    frame.render_widget(block, rect);

    let parts = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Length(1), Constraint::Min(1)])
        .split(inner);

    // query line
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("› ", Style::default().fg(DIM)),
            Span::raw(p.query.clone()),
            Span::styled("▏", Style::default().fg(ac)),
            Span::styled(
                format!("   {} matches", p.results.len()),
                Style::default().fg(DIM),
            ),
        ])),
        parts[0],
    );
    frame.render_widget(
        Paragraph::new(Span::styled("─".repeat(parts[1].width as usize), Style::default().fg(DIM))),
        parts[1],
    );

    if p.results.is_empty() {
        let msg = if p.mode == PickerMode::Search && p.query.trim().is_empty() {
            "  type to search note contents"
        } else {
            "  no matches"
        };
        frame.render_widget(
            Paragraph::new(Span::styled(msg, Style::default().fg(DIM))),
            parts[2],
        );
        return;
    }

    let items: Vec<ListItem> = p
        .results
        .iter()
        .enumerate()
        .map(|(i, r)| {
            let entry = &p.entries[r.idx];
            let bar = if i == p.sel {
                Span::styled(format!("{} ", g.sel), Style::default().fg(ac))
            } else {
                Span::raw("  ")
            };
            let name_style = if i == p.sel {
                Style::default().add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            let mut spans = vec![
                bar,
                Span::styled(format!("{} ", g.note), Style::default().fg(DIM)),
                Span::styled(entry.rel.clone(), name_style),
            ];
            if let Some(snip) = &r.snippet {
                spans.push(Span::styled(format!("   {snip}"), Style::default().fg(DIM)));
            }
            ListItem::new(Line::from(spans))
        })
        .collect();

    let list = List::new(items).highlight_style(Style::default());
    let mut state = ListState::default();
    state.select(Some(p.sel.min(p.results.len() - 1)));
    frame.render_stateful_widget(list, parts[2], &mut state);
}

fn render_tree(frame: &mut Frame, app: &App, area: Rect) {
    let g = glyphs();
    let ac = accent();
    // hairline on the right edge only — no full box
    let block = Block::default()
        .borders(Borders::RIGHT)
        .border_style(Style::default().fg(DIM))
        .padding(Padding::horizontal(1));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(vec![
        Span::styled(format!("{} ", g.brand), Style::default().fg(ac)),
        Span::styled("notakase", Style::default().fg(ac).add_modifier(Modifier::BOLD)),
    ]));
    lines.push(Line::from(""));

    // window the visible rows around the cursor so the tree scrolls
    let capacity = inner.height.saturating_sub(2) as usize;
    let total = app.rows.len();
    let start = if total <= capacity || capacity == 0 {
        0
    } else {
        app.cursor
            .saturating_sub(capacity / 2)
            .min(total - capacity)
    };

    for i in start..total.min(start + capacity.max(1)) {
        let row = app.rows[i];
        let node = &app.tree.nodes[row.node];
        let selected = i == app.cursor;

        let mut spans: Vec<Span> = Vec::new();
        spans.push(if selected {
            Span::styled(format!("{} ", g.sel), Style::default().fg(ac))
        } else {
            Span::raw("  ")
        });
        spans.push(Span::raw("  ".repeat(row.depth as usize)));

        if node.is_dir {
            // the open/closed folder icon carries the expand state on its own
            let icon = if node.expanded { g.folder_open } else { g.folder_closed };
            spans.push(Span::styled(format!("{icon}  "), Style::default().fg(ac)));
            let mut style = Style::default().add_modifier(Modifier::BOLD);
            if !selected {
                style = style.fg(Color::Reset);
            }
            spans.push(Span::styled(node.name.clone(), style));
        } else {
            spans.push(Span::styled(format!("{}  ", g.note), Style::default().fg(DIM)));
            let label = node.name.strip_suffix(".md").unwrap_or(&node.name).to_string();
            let style = if selected {
                Style::default().add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::Reset)
            };
            spans.push(Span::styled(label, style));
        }
        lines.push(Line::from(spans));
    }

    frame.render_widget(Paragraph::new(lines), inner);
}

fn render_preview(frame: &mut Frame, app: &App, area: Rect) {
    let g = glyphs();
    let ac = accent();
    let has_tree_border = app.show_tree && area.width >= 40;
    let block = if has_tree_border {
        Block::default().padding(Padding::new(2, 1, 0, 0))
    } else {
        Block::default().padding(Padding::horizontal(2))
    };
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let Some(idx) = app.selected() else {
        frame.render_widget(
            Paragraph::new(Span::styled("no note selected", Style::default().fg(DIM))),
            inner,
        );
        return;
    };
    let node = &app.tree.nodes[idx];

    // ---- fixed header (never scrolls): breadcrumb · title · meta ----
    let rel = node
        .path
        .strip_prefix(&app.root)
        .unwrap_or(&node.path)
        .to_string_lossy()
        .replace('/', "  ›  ");
    let title = node
        .name
        .strip_suffix(".md")
        .unwrap_or(&node.name)
        .to_string();

    let mut header: Vec<Line> = Vec::new();
    header.push(Line::from(""));
    header.push(Line::from(Span::styled(
        rel,
        Style::default().fg(DIM),
    )));
    let title_icon = if node.is_dir { g.folder_open } else { g.note };
    header.push(Line::from(vec![
        Span::styled(format!("{title_icon}  "), Style::default().fg(ac)),
        Span::styled(title, Style::default().fg(ac).add_modifier(Modifier::BOLD)),
    ]));
    if let Some((lines, words)) = app.preview_stats {
        header.push(Line::from(Span::styled(
            format!("{lines} lines · {words} words"),
            Style::default().fg(DIM),
        )));
    }
    header.push(Line::from(""));
    header.push(Line::from(Span::styled(
        "─".repeat(inner.width as usize),
        Style::default().fg(DIM),
    )));
    header.push(Line::from(""));

    let header_h = (header.len() as u16).min(inner.height);
    let parts = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(header_h), Constraint::Min(1)])
        .split(inner);
    frame.render_widget(Paragraph::new(header), parts[0]);

    // ---- scrollable body: rendered markdown (or folder summary) ----
    let body_area = parts[1];
    let content = app.preview.clone();

    let full = Paragraph::new(content.clone())
        .wrap(Wrap { trim: false })
        .line_count(body_area.width) as u16;

    if full > body_area.height {
        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Min(1), Constraint::Length(1)])
            .split(body_area);
        let text_area = cols[0];
        let para = Paragraph::new(content).wrap(Wrap { trim: false });
        let total = para.line_count(text_area.width) as u16;
        let max = total.saturating_sub(text_area.height);
        app.max_scroll.set(max);
        let offset = app.scroll.min(max);
        frame.render_widget(para.scroll((offset, 0)), text_area);

        let mut sb = ScrollbarState::new(total as usize)
            .position(offset as usize)
            .viewport_content_length(text_area.height as usize);
        frame.render_stateful_widget(
            Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .begin_symbol(None)
                .end_symbol(None)
                .thumb_style(Style::default().fg(ac))
                .track_style(Style::default().fg(DIM)),
            cols[1],
            &mut sb,
        );
    } else {
        app.max_scroll.set(0);
        frame.render_widget(Paragraph::new(content).wrap(Wrap { trim: false }), body_area);
    }
}

fn render_status_bar(frame: &mut Frame, app: &App, area: Rect) {
    let g = glyphs();
    let ac = accent();

    // An active prompt takes over the whole bar.
    if let Some(p) = &app.prompt {
        let mut spans = vec![
            Span::styled(format!(" {} ", p.kind.label()), Style::default().fg(ac).add_modifier(Modifier::BOLD)),
            Span::styled("› ", Style::default().fg(DIM)),
        ];
        if let crate::app::PromptKind::ConfirmDelete { path } = &p.kind {
            spans.push(Span::styled(format!("delete {path}? (y/N)"), Style::default().fg(Color::Red)));
        } else {
            spans.push(Span::raw(p.buf.clone()));
            spans.push(Span::styled("▏", Style::default().fg(ac)));
        }
        if let Some(err) = &p.err {
            spans.push(Span::styled(format!("   {err}"), Style::default().fg(Color::Red)));
        }
        frame.render_widget(Paragraph::new(Line::from(spans)), area);
        return;
    }

    let hint = if app.show_tree {
        " j/k move · a new · e edit · ^p find · / search · d delete · q quit"
    } else {
        " J/K scroll · ^p find · / search · Tab tree · q quit"
    };
    // A transient notice (e.g. "created X") replaces the hint until next key.
    let left = match &app.notice {
        Some(msg) => Span::styled(format!(" {msg}"), Style::default().fg(ac)),
        None => Span::styled(hint, Style::default().fg(DIM)),
    };

    // right side: sync indicators (folder / relay · lock · last result) + brand
    let mut right: Vec<Span> = Vec::new();
    let mut synced = false;
    if let Some(folder) = &app.sync_folder {
        right.push(Span::styled(format!("{} {}", g.cloud, folder), Style::default().fg(Color::Green)));
        synced = true;
    }
    if let Some(host) = &app.server {
        if synced {
            right.push(Span::raw("  "));
        }
        right.push(Span::styled(format!("{} {}", g.server, host), Style::default().fg(Color::Green)));
        synced = true;
    }
    if synced {
        if app.encrypted {
            right.push(Span::styled(format!("  {}", g.lock), Style::default().fg(Color::Green)));
        }
        if let Some(msg) = &app.sync_msg {
            right.push(Span::styled(format!("  {msg}"), Style::default().fg(DIM)));
        }
        right.push(Span::styled("   ", Style::default()));
    } else {
        right.push(Span::styled("local only   ", Style::default().fg(DIM)));
    }
    right.push(Span::styled(format!("{} notakase ", g.brand), Style::default().fg(ac)));

    let lw = left.content.chars().count();
    let rw: usize = right.iter().map(|s| s.content.chars().count()).sum();
    let pad = (area.width as usize).saturating_sub(lw + rw);
    let mut spans = vec![left, Span::raw(" ".repeat(pad))];
    spans.extend(right);
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

#[cfg(test)]
mod snapshot {
    use super::*;
    use crate::app::App;
    use crate::tree::Tree;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;
    use std::path::PathBuf;

    // Render a representative frame and print it. Run with:
    //   NOTAKASE_ASCII=1 cargo test render_snapshot -- --nocapture
    #[test]
    fn render_snapshot() {
        std::env::set_var("NOTAKASE_ASCII", "1");
        std::env::set_var("NOTAKASE_ACCENT", "cyan");
        let root = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../notes"));
        let tree = Tree::build(&root);
        let mut app = App::new(tree, root);
        // drill into Reference/ and land on the markdown showcase note
        while app
            .selected()
            .map(|i| app.tree.nodes[i].name != "Reference")
            .unwrap_or(false)
        {
            app.move_cursor(1);
        }
        app.expand(); // open Reference
        app.move_cursor(1); // keybindings.md
        app.move_cursor(1); // markdown-showcase.md
        app.refresh_preview();

        let mut terminal = Terminal::new(TestBackend::new(96, 38)).unwrap();
        terminal.draw(|f| render(f, &app)).unwrap();
        let buf = terminal.backend().buffer().clone();
        let mut out = String::from("\n");
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                out.push_str(buf[(x, y)].symbol());
            }
            out.push('\n');
        }
        println!("{out}");
    }
}
