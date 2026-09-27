# Data retention and archives

Every report is kept until you archive or delete it, so the database grows steadily (roughly one row per resource per probe interval). The `[retention]` settings keep that in check ([reference/configuration.md](../reference/configuration.md#retention)).

## Size warnings

Every 10 minutes the hub compares the database size with `warn_db_size_mb` (default 1000 MB). Over the limit, it logs a warning and the admin page shows it. With `auto = true` it also archives old data automatically.

## Archiving

Archiving moves reports older than a cutoff (default `archive_older_than_days` = 90) out of the main database into a separate file in `archive_path`:

1. The database is copied to `archives/blentinel_archive_<cutoff-date>.db`.
2. Newer data is removed from the copy, so it holds only old data.
3. The old data is removed from the main database, which is then compacted.
4. The archive is recorded, so the admin page lists it.

Start it from the admin page, with `POST /api/admin/archive` (optionally `{"cutoff_days": 30}`), or automatically with `auto = true`.

## Viewing archives

The admin page opens an archive in a read-only view with the same dashboard as live data.

> **Known issue (D-26):** the archive view shows almost nothing today. It hides probes that are still reporting, and its uptime chart is always empty. The archived data itself is intact in the archive file. Planned summary tables (proposal 0007) may replace archiving altogether (D-20).

## Deleting data

On the admin page, per company or per probe:

- **Delete data:** removes the reports; the probe stays known and keeps reporting.
- **Remove probe:** also forgets the probe. It reappears if it's still in `[[probes]]` and reports again. To stop a probe for good, remove it from `[[probes]]` too.
