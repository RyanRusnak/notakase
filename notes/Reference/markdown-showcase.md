# Markdown showcase

This note exists to show off the preview renderer. Everything below is themed
with your **active Omarchy palette** — headings and links take the accent, code
takes the theme's green, and quotes ride a dim gutter.

## Text styles

You can write **bold**, *italic*, ***both at once***, and ~~struck-through~~
text. Inline `code spans` render in the theme's code color. Links like
[the Omarchy site](https://omarchy.org) glow in the accent.

### A third-level heading

Hierarchy is carried by weight and color, not by loud `#` marks on screen.

## Lists

Unordered:

- Plain markdown files on disk
- No database, no proprietary format
  - Nesting works too
  - As deep as you like
- Sync with anything that syncs files

Ordered:

1. Point at a folder
2. Write markdown
3. There is no step three

Task lists:

- [x] Render a folder tree
- [x] Render markdown beautifully
- [ ] Fuzzy-find across notes
- [ ] Inline editing

## Quotes

> The best interface is one that gets out of the way.
>
> — every good tool, eventually

## Code

Inline: call `notakase --notes ~/Documents/notes` to point it anywhere.

Fenced block:

```rust
fn accent() -> Color {
    // Pulled straight from the live Omarchy theme's colors.toml.
    omarchy::accent().unwrap_or(Color::Magenta)
}
```

## A small table

| Pane    | Content              | Themed |
|---------|----------------------|--------|
| Left    | Folder tree          | yes    |
| Right   | Markdown preview     | yes    |
| Status  | Keybinding hints     | yes    |

---

That horizontal rule above is themed too. That's the whole showcase.
