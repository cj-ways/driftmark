---
status: accepted
date: 2026-10-18
---

# 0010 Pages follow the review-first design

> Short version: driftmark's pages use direction C from the design spike, "review first". Body text uses the device's system font, and diagrams use the unmodified Liberation Sans that the server measures with.

## Situation

The design spike (`docs/working/version-1/spikes/design.md`) built three directions on this repo's own documents: A calm reading, B dense reference, C review first. All three met the readability rules: body text 16 px or more, about 60 to 80 characters per line on desktop, WCAG AA contrast in both themes, change marks that never rely on colour alone, sticky table headers and phone cards. The maintainer looked at them on 2026-10-18 and picked C.

## Decision

1. **Direction C, review first.**
   - The home page is an inbox of branches with document changes.
   - A change page has a rail listing every change, and one card per change.
   - Each card has "Seen" and "Add a note"; the note itself arrives in M3.
   - The first screen says where to look.
   - Reading pages show which sections have changes waiting.
   - Body text is 17 px on desktop and phone.
2. **Body font: the system font stack.** It adds 0 bytes, and looks native on each device.
3. **Diagram font: the unmodified Liberation Sans, shipped whole.** It is 147 KB, downloaded once per device and cached. It covers Latin, Greek and Cyrillic, and needs no renaming under its licence, because it is not modified. A Latin-only subset (20.6 KB) was rejected: a modified copy must be renamed, and labels in other scripts could be cut.

## What this costs

- C is the busiest of the three directions. A reader who mostly reads, and rarely reviews, would have been better served by A.
- The first diagram page on a new device downloads 147 KB more.
