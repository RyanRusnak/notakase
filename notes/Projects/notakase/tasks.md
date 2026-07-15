# notakase — live tasks

This note embeds a **live task list** from todokase. The block below is a query,
not stored task data — it re-resolves against todokase every time you open the
note, so it's always current.

```todokase
project: notakase
status: open
```

## How the embed works

A fenced `todokase` code block accepts these keys:

| Key       | Meaning                                   | Default |
|-----------|-------------------------------------------|---------|
| `project` | project name (or id)                      | all     |
| `context` | a context like `@work` (with or without `@`) | any  |
| `status`  | `open`, `done`, or `all`                  | open    |
| `limit`   | max rows                                  | 100     |

Because it's a plain fenced block, other markdown viewers (Obsidian, GitHub)
just show it as code — it only comes alive inside notakase.
