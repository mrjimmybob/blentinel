# M6-01: Background Task Integration Guide

## Overview

M6-01 provides idempotent rollup computation and retention pruning via `hub/src/service/rollups_task.rs`. This document describes how to integrate it into the hub's background task infrastructure.

## Integration Points

### 1. Rollup Computation Task

**Function:** `service::rollups_task::run_rollup_compute(pool)`

**What it does:**
- Computes 5-minute, hourly, and daily rollups from raw check_results
- Idempotent: safe to call repeatedly (UPSERT overwrites identical buckets)
- Handles late-arriving data: incremental approach picks up new results on re-run

**When to call:**
- Every 1 hour (recommended: at :00 of each hour)
- Rationale: Ensures 5m rollups are complete for prior 5 windows, hourly for prior hour, daily for prior day

**Return value:** `RollupStats` (counts of computed buckets)

**Example:**
```rust
let stats = service::rollups_task::run_rollup_compute(pool).await?;
tracing::info!("Computed {} 5m, {} hourly, {} daily buckets", 
    stats.computed_5m_buckets, 
    stats.computed_hourly_buckets, 
    stats.computed_daily_buckets
);
```

### 2. Retention Pruning Task

**Function:** `service::rollups_task::run_retention_prune(pool)`

**What it does:**
- Deletes check_results older than 14 days
- Deletes check_rollups_5m older than 90 days
- Deletes check_rollups_hourly older than 2 years
- Never deletes daily rollups (storage negligible, audit trail)

**When to call:**
- Once per day (recommended: at 00:00 UTC)
- Rationale: Stable operation, gives full day for late-arriving reports to be picked up before deletion

**Return value:** `RetentionStats` (counts of deleted rows)

**Example:**
```rust
let stats = service::rollups_task::run_retention_prune(pool).await?;
tracing::info!("Deleted {} raw, {} 5m, {} hourly rows", 
    stats.deleted_raw_results, 
    stats.deleted_5m_rollups, 
    stats.deleted_hourly_rollups
);
```

### 3. Combined Task

**Function:** `service::rollups_task::run_combined_task(pool)`

**What it does:**
- Runs rollup computation
- Then runs retention pruning
- Both operations are atomic from task perspective (no interleaving risk)

**When to call:** Daily (e.g., at 01:00 UTC, after all reporting is in)

**Example:**
```rust
let (rollup_stats, retention_stats) = service::rollups_task::run_combined_task(pool).await?;
tracing::info!("M6-01 task: computed {} buckets, deleted {} rows",
    rollup_stats.total_buckets(),
    retention_stats.total_deleted()
);
```

## Where to Add

### Option A: Hub's Main Event Loop (Simplest)

If hub has a main loop that runs periodic tasks:

```rust
// In hub/src/server.rs or similar
loop {
    // ... other work ...
    
    if should_run_rollups() {  // e.g., every hour
        if let Err(e) = service::rollups_task::run_rollup_compute(pool).await {
            tracing::error!("Rollup computation failed: {e:?}");
        }
    }
    
    if should_run_retention() {  // e.g., daily at 00:00 UTC
        if let Err(e) = service::rollups_task::run_retention_prune(pool).await {
            tracing::error!("Retention pruning failed: {e:?}");
        }
    }
    
    sleep(Duration::from_secs(60)).await;
}
```

### Option B: Dedicated Background Task (Recommended)

Add as a separate tokio task spawned at hub startup:

```rust
// In hub/src/server.rs or similar
running.tasks.spawn({
    let pool = pool.clone();
    async move {
        loop {
            tokio::time::sleep(Duration::from_secs(3600)).await;  // Every hour
            match service::rollups_task::run_rollup_compute(&pool).await {
                Ok(stats) => tracing::info!("Rollups: {} buckets computed", stats.total_buckets()),
                Err(e) => tracing::error!("Rollup task failed: {e:?}"),
            }
        }
    }
});

running.tasks.spawn({
    let pool = pool.clone();
    async move {
        loop {
            // Wait until next midnight UTC
            let now = Utc::now();
            let next_midnight = (now + Duration::days(1))
                .with_hour(0).unwrap()
                .with_minute(0).unwrap()
                .with_second(0).unwrap()
                .with_nanosecond(0).unwrap();
            let wait = next_midnight.signed_duration_since(now);
            
            tokio::time::sleep(wait.to_std().unwrap()).await;
            
            match service::rollups_task::run_retention_prune(&pool).await {
                Ok(stats) => tracing::info!("Retention: {} rows deleted", stats.total_deleted()),
                Err(e) => tracing::error!("Retention task failed: {e:?}"),
            }
        }
    }
});
```

### Option C: APScheduler-style Cron Task (Future)

If hub adds a proper job scheduler library (e.g., `tokio_cron_scheduler`), use its cron syntax:

```rust
// Pseudo-code: requires adding dependency
scheduler.add(
    Job::new("0 * * * *", move |_| {  // Every hour at :00
        Box::pin(async {
            let _ = service::rollups_task::run_rollup_compute(&pool).await;
        })
    })?
)?;
```

## Error Handling

Both functions return `anyhow::Result<T>`. Errors are typically database issues. Recommended approach:

```rust
match service::rollups_task::run_combined_task(&pool).await {
    Ok((rollup_stats, retention_stats)) => {
        tracing::info!("M6-01 task succeeded: {} buckets, {} deletions",
            rollup_stats.total_buckets(),
            retention_stats.total_deleted()
        );
    }
    Err(e) => {
        tracing::error!("M6-01 task failed (will retry next run): {e:?}");
        // Do NOT panic — the error is transient (db timeout, lock, etc.)
        // The idempotent design means retry will pick up where it left off
    }
}
```

## Testing

### Manual Testing

```bash
# Set DATABASE_URL and run rollup computation
cargo run -p hub -- rollups-compute

# Run retention pruning
cargo run -p hub -- retention-prune
```

(Requires adding CLI subcommands to hub/src/cli.rs or similar)

### Automated Testing

Integration tests in `hub/tests/m6_01_integration.rs` (currently placeholders) will validate:
- Idempotency: running twice produces identical results
- Late arrivals: buffered reports picked up before pruning
- Retention boundaries: data preserved across tiers
- Cross-tier accuracy: availability % consistent across raw/5m/hourly/daily

Enable with:
```bash
export DATABASE_URL=sqlite:///tmp/test-m6-01.db
cargo test --test m6_01_integration -- --include-ignored
```

## Monitoring & Alerting

Recommended metrics to track:
- `m6_rollups_5m_buckets` — count of 5m buckets computed (should increase steadily)
- `m6_rollups_hourly_buckets` — count of hourly buckets computed
- `m6_rollups_daily_buckets` — count of daily buckets computed
- `m6_retention_raw_deleted` — rows pruned from raw check_results
- `m6_retention_5m_deleted` — rows pruned from 5m rollups
- `m6_retention_hourly_deleted` — rows pruned from hourly rollups
- `m6_task_duration_ms` — total time for compute + prune

Alert if:
- No buckets computed for 3 consecutive hours (stale data)
- Pruning deletes unexpected amount (e.g., 100x normal) → may indicate bug

## Dependencies

M6-01 tasks depend on:
- `hub/migrations/sqlite/0028_check_rollups.sql` — tables and config
- `hub/src/service/rollups.rs` — computation logic
- `hub/src/service/retention_policy.rs` — pruning logic
- `hub/src/service/rollups_task.rs` — task orchestration

All are included in PR #36; no additional dependencies.

## Timeline

- **PR #36:** M6-01 core ready for integration
- **M6-02:** Check timelines UI (uses tier_select queries)
- **M6-03:** Problems view (historical search)
- **M6-05:** SLA reports (availability_pct + printable HTML)
