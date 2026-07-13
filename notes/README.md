# notakase

A keyboard-first, terminal-native **notes** app for Omarchy — a sibling to
[todarchy](https://github.com/ryanrusnak/todarchy). Your notes are plain
markdown files in plain folders. No database, no lock-in, no cloud required.

> Notes are just files. The app is just a beautiful way to move through them.

## The idea

- **Left:** a tree of your folders you can drill into.
- **Right:** a live, themed markdown preview of whatever you land on.
- **Everywhere:** the colors come from your *active* Omarchy theme, so the app
  always matches your desktop.

## Try it

```bash
cargo run
```

Then move around:

| Key       | Does                          |
|-----------|-------------------------------|
| `j` / `k` | Move down / up                |
| `l` / `↵` | Expand a folder               |
| `h`       | Collapse / jump to parent     |
| `J` / `K` | Scroll the preview            |
| `Tab`     | Zen mode (hide the tree)      |
| `q`       | Quit                          |

Have a look at `Reference/markdown-showcase.md` to see the renderer stretch its
legs.
