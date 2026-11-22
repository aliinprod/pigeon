---
name: day-post
description: Draft the daily "Building Pigeon · Day N" post for X from DEVLOG.md and today's git history. Use at the end of each work day or when the owner asks for today's post. Produces a draft only; the owner posts it.
---

# Day post

1. Read the latest entry in `DEVLOG.md` and `git log --since=midnight --oneline`.
2. Pick ONE story for the day: a result with a number, a failure and its fix, or a decision and why.
3. Write the draft in English using this template, max 280 characters for the main post:

```
Building Pigeon 🕊️ · Day N

<one-line result, with a number if possible>

<what broke or what was learned, one line>

Next: <tomorrow in a few words>
```

4. Suggest the visual: which screenshot, terminal output or GIF to attach (follow `brand/BRAND.md`).
5. If the day had a deeper topic (architecture, a hard bug), add an optional 3–5 post thread draft.
6. Rules: honest, concrete, no hype words, no private data (no real file names from the owner's work, no IPs, no node IDs, no company names).
7. Save the draft to `posts/day-N.md` and show it to the owner. Never post anything yourself.
