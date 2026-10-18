---
status: accepted
---

# Page cache

> Short version: the server keeps rendered pages for a while, so a second visit is fast.

## Decision

Keep each rendered page for one hour, then drop it.

- Pages with open notes expire sooner.
- The cache never holds more than 64 MB.
