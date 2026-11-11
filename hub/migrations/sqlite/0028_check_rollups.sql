-- M6-01: Historical data foundation (proposal 0007, D-10, D-110-rc-scope)
-- Rollups: 5-min / hourly / daily aggregations of check results for efficient historical queries
-- Retention: configurable per tier (raw 14d, 5m 90d, hourly 2y, daily forever)
-- Enables: M6-02 (timelines), M6-05 (reports), dashboard trends

-- 5-minute rollups: aggregate results per 5-min bucket
CREATE TABLE check_rollups_5m (
    check_id TEXT NOT NULL,
    bucket_start DATETIME NOT NULL,  -- e.g. 2026-11-08 14:35:00 (top of 5-min bucket)
    status_up INTEGER NOT NULL DEFAULT 0,
    status_warn INTEGER NOT NULL DEFAULT 0,
    status_down INTEGER NOT NULL DEFAULT 0,
    status_unknown INTEGER NOT NULL DEFAULT 0,
    latency_ms_min INTEGER,
    latency_ms_avg REAL,
    latency_ms_max INTEGER,
    count_reports INTEGER NOT NULL DEFAULT 0,
    computed_at DATETIME NOT NULL,
    PRIMARY KEY (check_id, bucket_start)
);

CREATE INDEX idx_check_rollups_5m_check ON check_rollups_5m(check_id, bucket_start DESC);

-- Hourly rollups
CREATE TABLE check_rollups_hourly (
    check_id TEXT NOT NULL,
    bucket_start DATETIME NOT NULL,  -- e.g. 2026-11-08 14:00:00 (top of hour)
    status_up INTEGER NOT NULL DEFAULT 0,
    status_warn INTEGER NOT NULL DEFAULT 0,
    status_down INTEGER NOT NULL DEFAULT 0,
    status_unknown INTEGER NOT NULL DEFAULT 0,
    latency_ms_min INTEGER,
    latency_ms_avg REAL,
    latency_ms_max INTEGER,
    count_reports INTEGER NOT NULL DEFAULT 0,
    computed_at DATETIME NOT NULL,
    PRIMARY KEY (check_id, bucket_start)
);

CREATE INDEX idx_check_rollups_hourly_check ON check_rollups_hourly(check_id, bucket_start DESC);

-- Daily rollups
CREATE TABLE check_rollups_daily (
    check_id TEXT NOT NULL,
    bucket_start DATETIME NOT NULL,  -- e.g. 2026-11-08 00:00:00 (start of day UTC)
    status_up INTEGER NOT NULL DEFAULT 0,
    status_warn INTEGER NOT NULL DEFAULT 0,
    status_down INTEGER NOT NULL DEFAULT 0,
    status_unknown INTEGER NOT NULL DEFAULT 0,
    latency_ms_min INTEGER,
    latency_ms_avg REAL,
    latency_ms_max INTEGER,
    count_reports INTEGER NOT NULL DEFAULT 0,
    computed_at DATETIME NOT NULL,
    PRIMARY KEY (check_id, bucket_start)
);

CREATE INDEX idx_check_rollups_daily_check ON check_rollups_daily(check_id, bucket_start DESC);

-- Retention policy: configurable per tier, applied daily
CREATE TABLE retention_config (
    tier TEXT PRIMARY KEY,
    retention_days INTEGER NOT NULL,
    last_pruned_at DATETIME
);

-- Initialize retention config with RC defaults (D-110-rc-scope)
INSERT INTO retention_config (tier, retention_days) VALUES
    ('raw', 14),          -- raw check_results: 14 days (daily monitoring, incident investigation)
    ('5m', 90),           -- 5-min rollups: 90 days (3 months, weekly patterns)
    ('hourly', 730),      -- hourly rollups: 2 years (long-term trends, seasonal patterns, SLA history)
    ('daily', 36500);     -- daily rollups: 100 years (effectively forever; storage negligible)
