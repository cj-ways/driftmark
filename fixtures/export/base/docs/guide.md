---
status: current
---

# Field guide

> Short version: how the made-up harbour team keeps its lanterns lit.

## Install

Fetch the lantern kit, then read the [cache decision](decisions/0001-cache.md#decision) and the [old rule](decisions/0002-old.md).

| Step | Who | When |
|---|---|---|
| Fill the oil | Keeper | Dusk |
| Trim the wick | Keeper | Midnight |

```sh
lantern fill --all
```

```rust
fn main() {
    println!("lit");
}
```

```mermaid
flowchart LR
  A[Dusk] --> B[Light the lantern]
```

> [!WARNING]
> Never leave a lantern alone in a storm.

See the [source](../src/lantern.rs), the ![harbour map](map.png) and a [missing page](nowhere.md). Tides matter[^tides].

[^tides]: Tides change the schedule twice a day.

## Night watch

The lighthouse keeper writes a storm log entry every night.
