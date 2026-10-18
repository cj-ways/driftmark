# Audit rows

## Retention

Keep audit rows for thirty days, then archive them with the nightly job.

Rows older than a year move to cold storage; nothing is deleted any more.

## Access

Only the support and security teams read audit rows, through the admin pages.
