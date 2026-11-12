-- M6-01: history rollups, rebuilt (proposal 0007 §2, D-111).
--
-- 0028's tables are replaced: the job that filled them never ran (it read a
-- column `reports` doesn't have), so they hold nothing. Rollups are now added
-- in the transaction that stores each report, so they always match the raw
-- results, and raw results can be deleted without losing history.

DROP TABLE check_rollups_5m;
DROP TABLE check_rollups_hourly;
DROP TABLE check_rollups_daily;
DROP TABLE retention_config;

-- Per check and bucket: how many results had each status. Results during a
-- maintenance window count only in `maintenance` (D-65). Latency is kept as a
-- sum and a count, so a coarser bucket's average is exact.
CREATE TABLE check_rollups (
    width         INTEGER NOT NULL,   -- bucket width in seconds: 300, 3600 or 86400
    bucket_start  INTEGER NOT NULL,   -- Unix seconds (UTC), a multiple of width
    company_id    TEXT    NOT NULL,   -- as stored with the report
    probe_id      TEXT    NOT NULL,
    check_id      TEXT    NOT NULL,
    up            INTEGER NOT NULL DEFAULT 0,
    warning       INTEGER NOT NULL DEFAULT 0,
    down          INTEGER NOT NULL DEFAULT 0,
    unknown       INTEGER NOT NULL DEFAULT 0,
    maintenance   INTEGER NOT NULL DEFAULT 0,
    latency_min   INTEGER,
    latency_max   INTEGER,
    latency_sum   INTEGER NOT NULL DEFAULT 0,
    latency_count INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (width, check_id, bucket_start, company_id)
) WITHOUT ROWID;

CREATE INDEX idx_check_rollups_company ON check_rollups(width, company_id, bucket_start);
CREATE INDEX idx_check_rollups_age     ON check_rollups(width, bucket_start);
CREATE INDEX idx_check_rollups_probe   ON check_rollups(probe_id);

-- Per check, metric and bucket: min, max, sum and count of its values.
CREATE TABLE metric_rollups (
    width        INTEGER NOT NULL,
    bucket_start INTEGER NOT NULL,
    company_id   TEXT    NOT NULL,
    probe_id     TEXT    NOT NULL,
    check_id     TEXT    NOT NULL,
    metric_key   TEXT    NOT NULL,
    unit         TEXT    NOT NULL,    -- the latest seen
    value_min    REAL    NOT NULL,
    value_max    REAL    NOT NULL,
    value_sum    REAL    NOT NULL,
    value_count  INTEGER NOT NULL,
    PRIMARY KEY (width, check_id, metric_key, bucket_start, company_id)
) WITHOUT ROWID;

CREATE INDEX idx_metric_rollups_age   ON metric_rollups(width, bucket_start);
CREATE INDEX idx_metric_rollups_probe ON metric_rollups(probe_id);

-- How long each tier is kept (at most one row). Daily rollups are kept forever.
CREATE TABLE history_retention (
    id               INTEGER PRIMARY KEY CHECK (id = 1),
    raw_days         INTEGER NOT NULL,
    five_minute_days INTEGER NOT NULL,
    hourly_days      INTEGER NOT NULL,
    updated_by       TEXT,
    updated_at       DATETIME
);
