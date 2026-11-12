# Data retention and archives

## History and how long it is kept

When the hub stores a report, it also adds the report's results to three summaries of each check: one per **5 minutes**, one per **hour**, and one per **day**. Each summary counts the Up, Warning, Down and Unknown results and the results during maintenance windows, the lowest, highest and average latency, and the lowest, highest and average of each metric. The summaries are updated with the report itself, so they are never behind and never miss a result (D-111).

Older detail is then let go, every hour:

| What | Kept by default |
|---|---|
| Reports and their raw results | 14 days (each probe's newest report is always kept) |
| 5-minute summaries | 90 days |
| Hourly summaries | 2 years |
| Daily summaries | forever |

The uptime chart reads the summaries, so its 30-day and "all" views still show everything after the raw results are gone.

An Admin or Owner changes these periods in **Admin › History**: type the days for raw results, 5-minute and hourly summaries, then **Save**. The same is `POST /api/admin/history-retention` ([reference/api.md](../reference/api.md#admin)). Each period is 1 to 36,500 days, and a coarser summary is kept at least as long as a finer one; the page refuses anything else before sending it, with the hub's reason. A change is in the audit log (`history.retention`) and applies at the next hourly run.

Shortening a period deletes the older detail at that run; it can't be brought back. The daily summaries stay, so the uptime chart still covers the whole history, in coarser steps.

The database still grows with the number of checks, but no longer without limit. The `[retention]` settings below (size warning, archiving) still apply ([reference/configuration.md](../reference/configuration.md#retention)).

## Size warnings

Every 10 minutes the hub compares the database size with `warn_db_size_mb` (default 1000 MB). Over the limit, it logs a warning and the admin page shows it. With `auto = true` it also archives old data automatically.

## Archiving

Archiving moves reports older than a cutoff (default `archive_older_than_days` = 90) out of the main database into a separate file in `archive_path`:

1. The database is copied to `archives/blentinel_archive_<cutoff-date>.db`.
2. Newer data is removed from the copy, so it holds only reports from before the cutoff. It keeps every probe it has reports from, including probes that are still reporting today, shown as last seen at their last archived report.
3. The old data is removed from the main database, which is then compacted.
4. The archive is recorded, so the admin page lists it.

The main database keeps its summaries of the archived period, so its uptime chart ("all") still shows it. The archive works out its own summaries from the reports it holds. With the default 14-day raw retention, an archive of reports older than 90 days holds only each probe's last report from that time: the summaries in the main database are the long-term history.

Start it from the admin page, with `POST /api/admin/archive` (optionally `{"cutoff_days": 30}`), or automatically with `auto = true`.

## Viewing archives

The admin page opens an archive in a read-only view with the same dashboard as live data. Click a company to see its probes, their devices at the time of their last archived report, and the uptime chart. Silencing isn't available in an archive.

The uptime ranges (24 hours, 7 days, 30 days, all) end at the archive's **newest report**, not today, because an archive holds only data from before its cutoff. "Last 7 days" is the last 7 days of the archive.

Deleting old data outright (purge) is planned (ROADMAP M1-10); archiving stays (D-39).

## Deleting data

On the admin page, per company or per probe:

- **Delete data:** removes the reports and their history summaries; the probe stays known and keeps reporting.
- **Remove probe:** also forgets the probe and its enrollment: its reports are refused (`unknown_probe`) until it enrolls again. To keep its history and only stop it, **Revoke** it on its company's page instead.
