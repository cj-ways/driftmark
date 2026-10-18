# Working

> Short version: active work, one folder per topic. Each folder has a plan and a status file. When the work ends, the whole folder moves to the archive.

## What belongs here

- `docs/working/<topic>/plan.md`: the goal and the steps.
- `docs/working/<topic>/status.md`: where the work stands, what comes next, and a log.
- Notes, findings and drafts that the work needs.

## What does not belong here

- Results that must outlast the work. Before the work ends, move them into [a feature](../features/), [a standard](../standards/) or [a decision](../decisions/).

## Starting work

Run both commands with the same topic:

```sh
driftmark new plan "Faster search"
driftmark new status "Faster search"
```

They make `docs/working/faster-search/plan.md` and `docs/working/faster-search/status.md`.

## Statuses

| Status | Meaning |
|---|---|
| `active` | The work is going on. |
| `paused` | The work has stopped for now. The status file says why. |
| `done` | The work is finished, and the folder can move to the archive. |

Set `date` in the status file each time you add to its log.

## Ending work

1. Move what must last into features, standards or decisions.
2. Set `status: done` in the plan and in the status file.
3. Move the folder, under the same name: `git mv docs/working/faster-search docs/archive/faster-search`.
