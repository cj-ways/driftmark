---
status: accepted
date: 2026-11-02
---

# Lantern cache lifetime

> Short version: how long the made-up lantern server keeps a rendered page. Throwaway text for the M5 live check.

## Decision

Keep each rendered lantern page in the cache for 90 minutes.

Never cache a page while a reviewer has it open.
