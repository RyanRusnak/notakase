# Paper notes

Proof that folders nest as deep as a project needs — this note lives six levels
down:

```
Projects/client/2026/q3/research/sources/paper-notes.md
```

The tree on the left drills all the way in, and the backend stores this note's
**full relative path** inside its CRDT document, so the depth survives sync and
device moves.

## Takeaways

- [x] Deep nesting works front-to-back
- [ ] Summarize for the Q3 review
