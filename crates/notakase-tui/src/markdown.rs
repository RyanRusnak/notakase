// markdown.rs — render a markdown note into styled ratatui lines.
//
// Everything is expressed in ANSI/indexed colors + the app accent, so the
// output recolors with whatever Omarchy theme the terminal is running:
// headings and links use the accent, code uses the theme's green, quotes and
// rules use the dim (bright-black) slot. No hardcoded RGB.
//
// Handles: headings (weight-based hierarchy + an accent rule under H1/H2),
// bold/italic/strikethrough, inline + fenced code, ordered/unordered/nested
// lists, task-list checkboxes, blockquotes, links, images, horizontal rules,
// and GFM tables.

use pulldown_cmark::{Alignment, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

const DIM: Color = Color::DarkGray;
const CODE: Color = Color::Green;

/// Parse `md` and return themed lines ready to drop into a Paragraph.
pub fn render(md: &str, accent: Color) -> Vec<Line<'static>> {
    let mut opts = Options::empty();
    opts.insert(Options::ENABLE_STRIKETHROUGH);
    opts.insert(Options::ENABLE_TABLES);
    opts.insert(Options::ENABLE_TASKLISTS);
    let parser = Parser::new_ext(md, opts);

    let mut r = Renderer::new(accent);
    for ev in parser {
        r.event(ev);
    }
    r.finish()
}

struct Renderer {
    accent: Color,
    lines: Vec<Line<'static>>,
    cur: Vec<Span<'static>>,
    bold: i32,
    italic: i32,
    strike: i32,
    link: bool,
    image: bool,
    heading: Option<HeadingLevel>,
    heading_width: usize,
    lists: Vec<Option<u64>>,
    in_item: bool,
    quote: i32,
    in_code_block: bool,
    // table state
    table: Option<Table>,
}

struct Table {
    aligns: Vec<Alignment>,
    rows: Vec<Vec<String>>,
    cur_row: Vec<String>,
    cur_cell: String,
    in_head: bool,
    head_rows: usize,
}

impl Renderer {
    fn new(accent: Color) -> Self {
        Renderer {
            accent,
            lines: Vec::new(),
            cur: Vec::new(),
            bold: 0,
            italic: 0,
            strike: 0,
            link: false,
            image: false,
            heading: None,
            heading_width: 0,
            lists: Vec::new(),
            in_item: false,
            quote: 0,
            in_code_block: false,
            table: None,
        }
    }

    /// Style for the current inline context.
    fn inline_style(&self) -> Style {
        if let Some(level) = self.heading {
            // hierarchy by weight + color, not by visible '#' marks
            return match level {
                HeadingLevel::H1 => Style::default()
                    .fg(self.accent)
                    .add_modifier(Modifier::BOLD),
                HeadingLevel::H2 => Style::default().fg(self.accent).add_modifier(Modifier::BOLD),
                HeadingLevel::H3 => Style::default().add_modifier(Modifier::BOLD),
                _ => Style::default().fg(DIM).add_modifier(Modifier::BOLD),
            };
        }
        let mut s = Style::default();
        if self.link || self.image {
            s = s.fg(self.accent).add_modifier(Modifier::UNDERLINED);
        } else if self.quote > 0 {
            s = s.fg(DIM).add_modifier(Modifier::ITALIC);
        }
        if self.bold > 0 {
            s = s.add_modifier(Modifier::BOLD);
        }
        if self.italic > 0 {
            s = s.add_modifier(Modifier::ITALIC);
        }
        if self.strike > 0 {
            s = s.add_modifier(Modifier::CROSSED_OUT);
        }
        s
    }

    /// Prefix a freshly-started line with blockquote bars / list indent.
    fn line_prefix(&mut self) {
        for _ in 0..self.quote {
            self.cur
                .push(Span::styled("▏ ".to_string(), Style::default().fg(self.accent)));
        }
        if self.in_item {
            let depth = self.lists.len().saturating_sub(1);
            if depth > 0 {
                self.cur.push(Span::raw("  ".repeat(depth)));
            }
        }
    }

    fn flush(&mut self) {
        let spans = std::mem::take(&mut self.cur);
        self.lines.push(Line::from(spans));
    }

    fn blank(&mut self) {
        if self.lines.last().map(|l| l.spans.is_empty()).unwrap_or(false) {
            return;
        }
        self.lines.push(Line::from(""));
    }

    fn event(&mut self, ev: Event) {
        match ev {
            Event::Start(tag) => self.start(tag),
            Event::End(tag) => self.end(tag),
            Event::Text(t) => self.text(&t),
            Event::Code(t) => {
                if self.table.is_some() {
                    if let Some(tb) = self.table.as_mut() {
                        tb.cur_cell.push_str(&t);
                    }
                    return;
                }
                self.cur
                    .push(Span::styled(format!(" {t} "), Style::default().fg(CODE).bg(Color::Indexed(0))));
            }
            Event::SoftBreak | Event::HardBreak => {
                if self.table.is_some() {
                    if let Some(tb) = self.table.as_mut() {
                        tb.cur_cell.push(' ');
                    }
                    return;
                }
                self.flush();
                self.line_prefix();
            }
            Event::Rule => {
                self.blank();
                self.lines.push(Line::from(Span::styled(
                    "╌".repeat(32),
                    Style::default().fg(DIM),
                )));
                self.blank();
            }
            Event::TaskListMarker(done) => {
                // replace the bullet the Item start pushed with a checkbox
                self.cur.pop();
                let g = crate::theme::glyphs();
                let (glyph, color) = if done {
                    (g.task_done, CODE)
                } else {
                    (g.task_open, DIM)
                };
                self.cur
                    .push(Span::styled(format!("{glyph} "), Style::default().fg(color)));
            }
            _ => {}
        }
    }

    fn start(&mut self, tag: Tag) {
        match tag {
            Tag::Paragraph => {
                if !self.in_item {
                    self.line_prefix();
                }
            }
            Tag::Heading { level, .. } => {
                self.blank();
                self.heading = Some(level);
                self.heading_width = 0;
                // a leading accent tick gives headings presence without '#'
                if matches!(level, HeadingLevel::H1 | HeadingLevel::H2) {
                    self.cur
                        .push(Span::styled("▍ ".to_string(), Style::default().fg(self.accent)));
                }
            }
            Tag::BlockQuote(_) => {
                self.quote += 1;
            }
            Tag::CodeBlock(_) => {
                self.in_code_block = true;
                self.blank();
            }
            Tag::List(start) => {
                self.lists.push(start);
            }
            Tag::Item => {
                self.in_item = true;
                self.line_prefix();
                let marker = match self.lists.last_mut() {
                    Some(Some(n)) => {
                        let m = format!("{n}. ");
                        *n += 1;
                        m
                    }
                    _ => "• ".to_string(),
                };
                self.cur
                    .push(Span::styled(marker, Style::default().fg(self.accent)));
            }
            Tag::Emphasis => self.italic += 1,
            Tag::Strong => self.bold += 1,
            Tag::Strikethrough => self.strike += 1,
            Tag::Link { .. } => {
                self.link = true;
                let g = crate::theme::glyphs();
                self.cur
                    .push(Span::styled(format!("{} ", g.link), Style::default().fg(self.accent)));
            }
            Tag::Image { .. } => {
                self.image = true;
                let g = crate::theme::glyphs();
                self.cur
                    .push(Span::styled(format!("{} ", g.image), Style::default().fg(self.accent)));
            }
            Tag::Table(aligns) => {
                self.blank();
                self.table = Some(Table {
                    aligns,
                    rows: Vec::new(),
                    cur_row: Vec::new(),
                    cur_cell: String::new(),
                    in_head: false,
                    head_rows: 0,
                });
            }
            Tag::TableHead => {
                if let Some(tb) = self.table.as_mut() {
                    tb.in_head = true;
                }
            }
            Tag::TableCell => {
                if let Some(tb) = self.table.as_mut() {
                    tb.cur_cell.clear();
                }
            }
            _ => {}
        }
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph => {
                if !self.in_item {
                    self.flush();
                    self.blank();
                }
            }
            TagEnd::Heading(level) => {
                self.flush();
                // an accent rule under the big headings
                if matches!(level, HeadingLevel::H1 | HeadingLevel::H2) {
                    let w = (self.heading_width + 2).clamp(6, 40);
                    self.lines.push(Line::from(Span::styled(
                        "─".repeat(w),
                        Style::default().fg(self.accent),
                    )));
                }
                self.heading = None;
                self.blank();
            }
            TagEnd::BlockQuote(_) => {
                self.quote -= 1;
                self.blank();
            }
            TagEnd::CodeBlock => {
                self.in_code_block = false;
                self.blank();
            }
            TagEnd::List(_) => {
                self.lists.pop();
                if self.lists.is_empty() {
                    self.blank();
                }
            }
            TagEnd::Item => {
                self.in_item = false;
                self.flush();
            }
            TagEnd::Emphasis => self.italic -= 1,
            TagEnd::Strong => self.bold -= 1,
            TagEnd::Strikethrough => self.strike -= 1,
            TagEnd::Link => self.link = false,
            TagEnd::Image => self.image = false,
            TagEnd::TableCell => {
                if let Some(tb) = self.table.as_mut() {
                    let cell = tb.cur_cell.trim().to_string();
                    tb.cur_row.push(cell);
                }
            }
            TagEnd::TableHead | TagEnd::TableRow => {
                if let Some(tb) = self.table.as_mut() {
                    let row = std::mem::take(&mut tb.cur_row);
                    if tb.in_head {
                        tb.head_rows += 1;
                        tb.in_head = false;
                    }
                    tb.rows.push(row);
                }
            }
            TagEnd::Table => {
                if let Some(tb) = self.table.take() {
                    self.render_table(tb);
                }
                self.blank();
            }
            _ => {}
        }
    }

    fn text(&mut self, t: &str) {
        if let Some(tb) = self.table.as_mut() {
            tb.cur_cell.push_str(t);
            return;
        }
        if self.in_code_block {
            let parts: Vec<&str> = t.split('\n').collect();
            let last = parts.len().saturating_sub(1);
            for (i, line) in parts.iter().enumerate() {
                if i > 0 {
                    self.flush();
                }
                // the final newline yields a trailing empty segment — drop it
                // so it can't bleed into the next block's first line
                if i == last && line.is_empty() {
                    continue;
                }
                self.cur
                    .push(Span::styled("▏ ".to_string(), Style::default().fg(DIM)));
                self.cur
                    .push(Span::styled(line.to_string(), Style::default().fg(CODE)));
            }
            return;
        }
        if self.heading.is_some() {
            self.heading_width += t.chars().count();
        }
        let style = self.inline_style();
        self.cur.push(Span::styled(t.to_string(), style));
    }

    fn render_table(&mut self, tb: Table) {
        if tb.rows.is_empty() {
            return;
        }
        let cols = tb.rows.iter().map(|r| r.len()).max().unwrap_or(0);
        let mut widths = vec![0usize; cols];
        for row in &tb.rows {
            for (i, cell) in row.iter().enumerate() {
                widths[i] = widths[i].max(cell.chars().count());
            }
        }

        for (ri, row) in tb.rows.iter().enumerate() {
            let is_head = ri < tb.head_rows;
            let mut spans: Vec<Span> = Vec::new();
            for i in 0..cols {
                let cell = row.get(i).map(String::as_str).unwrap_or("");
                let align = tb.aligns.get(i).copied().unwrap_or(Alignment::None);
                let padded = pad(cell, widths[i], align);
                let style = if is_head {
                    Style::default().fg(self.accent).add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                };
                if i > 0 {
                    spans.push(Span::styled(" │ ".to_string(), Style::default().fg(DIM)));
                }
                spans.push(Span::styled(padded, style));
            }
            self.lines.push(Line::from(spans));

            // a dim rule under the header
            if is_head && ri + 1 == tb.head_rows {
                let mut rule: Vec<Span> = Vec::new();
                for (i, w) in widths.iter().enumerate() {
                    if i > 0 {
                        rule.push(Span::styled("─┼─".to_string(), Style::default().fg(DIM)));
                    }
                    rule.push(Span::styled("─".repeat(*w), Style::default().fg(DIM)));
                }
                self.lines.push(Line::from(rule));
            }
        }
    }

    fn finish(mut self) -> Vec<Line<'static>> {
        if !self.cur.is_empty() {
            self.flush();
        }
        while self.lines.last().map(|l| l.spans.is_empty()).unwrap_or(false) {
            self.lines.pop();
        }
        self.lines
    }
}

fn pad(s: &str, width: usize, align: Alignment) -> String {
    let len = s.chars().count();
    if len >= width {
        return s.to_string();
    }
    let gap = width - len;
    match align {
        Alignment::Right => format!("{}{}", " ".repeat(gap), s),
        Alignment::Center => {
            let left = gap / 2;
            let right = gap - left;
            format!("{}{}{}", " ".repeat(left), s, " ".repeat(right))
        }
        _ => format!("{}{}", s, " ".repeat(gap)),
    }
}
