# notakase — design spec

A notes companion to todarchy. Same principles, same aesthetic, same respect
for plain files.

## Principles

1. **Files are the source of truth.** A note is a `.md` file. A folder is a
   folder. You can `grep`, `git`, and `rsync` all of it.
2. **The theme is the desktop's theme.** Colors come from the active Omarchy
   palette so the app is never visually out of place.
3. **Keyboard first.** Everything from the home row, like todarchy.
4. **Airy, not boxy.** Whitespace and hairlines over heavy borders.

## Layout

```
┌ tree ──────┬ preview ─────────────────┐
│ Journal    │ # Today                  │
│ Projects   │                          │
│  notakase  │ A themed markdown render │
│ Reference  │ of the selected note.    │
└────────────┴──────────────────────────┘
```

## Open questions

- [ ] Inline editing, or always hand off to `$EDITOR`?
- [ ] Full-text search — `ripgrep` shell-out or an index?
- [ ] Frontmatter: surface `title` / `tags` in the preview header?

> Ship the prototype first. Decide the rest once it feels good to move around.
