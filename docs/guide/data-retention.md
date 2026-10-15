# Data retention and archives

Every report is kept until you archive or delete it, so the database grows steadily (roughly one row per resource per probe interval). The `[retention]` settings keep that in check ([reference/configuration.md](../reference/configuration.md#retention)).

## Size warnings

Every 10 minutes the hub compares the database size with `warn_db_size_mb` (default 1000 MB). Over the limit, it logs a warning and the admin page shows it. With `auto = true` it also archives old data automatically.

## Archiving

Archiving moves reports older than a cutoff (default `archive_older_than_days` = 90) out of the main database into a separate file in `archive_path`:

1. The database is copied to `archives/blentinel_archive_<cutoff-date>.db`.
2. Newer data is removed from the copy, so it holds only reports from before the cutoff. It keeps every probe it has reports from, including probes that are still reporting today, shown as last seen at their last archived report.
3. The old data is removed from the main database, which is then compacted.
4. The archive is recorded, so the admin page lists it.

Start it from the admin page, with `POST /api/admin/archive` (optionally `{"cutoff_days": 30}`), or automatically with `auto = true`.

## Viewing archives

The admin page opens an archive in a read-only view with the same dashboard as live data. Click a company to see its probes, their devices at the time of their last archived report, and the uptime chart. Silencing isn't available in an archive.

The uptime ranges (24 hours, 7 days, 30 days, all) end at the archive's **newest report**, not today, because an archive holds only data from before its cutoff. "Last 7 days" is the last 7 days of the archive.

Deleting old data outright (purge) is planned (ROADMAP M1-10); archiving stays (D-39).

## Deleting data

On the admin page, per company or per probe:

- **Delete data:** removes the reports; the probe stays known and keeps reporting.
- **Remove probe:** also forgets the probe and its enrollment: its reports are refused (`unknown_probe`) until it enrolls again. To keep its history and only stop it, **Revoke** it on its company's page instead.
