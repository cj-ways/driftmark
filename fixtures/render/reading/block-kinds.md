---
status: accepted
date: 2026-10-18
---

# Cache rendered pages

> Short version: the server keeps each rendered page for one hour, so a second visit costs nothing.

## Decision

Keep each page for **one hour**, then drop it. Pages with *open notes*
expire sooner, and ~~two hours~~ is too long.

Escapes \*stay\* as written, entities such as &amp; and &#x1F600; show as
characters, and a hard break ends this line\
before the next one.

- A tight list item.
- Another with `inline code`.

---

> A block quote.
>
> > A nested quote with **bold** words.

| Page | Time |
|---|---:|
| Home | 10 min |

```sh
cache build --release
```

    indented code

Setext heading
--------------

[guide]: https://example.com/guide
