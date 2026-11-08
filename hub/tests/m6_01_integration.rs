#![cfg(feature = "ssr")]
//! M6-01 integration tests: rollup boundaries, idempotency, late arrivals, availability across tiers

use chrono::{Duration, Utc};
use sqlx::sqlite::SqlitePoolOptions;

#[tokio::test]
#[ignore]  // Requires DATABASE_URL and schema migration
async fn test_rollup_5m_boundary() {
    // Setup: create pool, run migrations
    // Create check_results at minute boundaries: :05, :10, :15 etc
    // Run compute_5m_rollups
    // Assert each bucket has correct aggregates
    // Assert no cross-bucket contamination
    todo!("Implement with test database");
}

#[tokio::test]
#[ignore]
async fn test_rollup_idempotency() {
    // Setup: create pool, test data
    // Run compute_5m_rollups twice with same data
    // Assert second run produces identical results
    // Assert no duplicate rows or corruption
    todo!("Implement with test database");
}

#[tokio::test]
#[ignore]
async fn test_late_arriving_data_before_pruning() {
    // Setup: create pool, retention config (14d raw, 90d 5m)
    // T=0: Insert data at T-20 days
    // Run rollups (compute 5m from raw)
    // T+1: Add late-arriving data at T-19 days (within raw window, outside 5m)
    // Run rollups again
    // Assert late data is picked up in 5m rollups
    // Prune raw data older than 14d
    // Assert 5m rollups still contain aggregate from late data
    todo!("Implement with test database");
}

#[tokio::test]
#[ignore]
async fn test_availability_across_tiers() {
    // Setup: create pool, test data spanning 90+ days
    // Populate raw check_results (Status: Up/Down/Unknown)
    // Run rollups (5m, hourly, daily)
    // Query availability_pct for 365-day range (uses daily rollups)
    // Query availability_pct for 30-day range (uses 5m rollups)
    // Query availability_pct for 6-hour range (uses raw)
    // Assert all three return same % (within rounding)
    todo!("Implement with test database");
}

#[tokio::test]
#[ignore]
async fn test_retention_respects_boundaries() {
    // Setup: retention config (14d raw, 90d 5m, 730d hourly)
    // T=0: Insert data at T-100 days (outside raw window, inside 5m/hourly)
    // Run rollups
    // Prune old raw (delete >14d)
    // Assert raw deleted, but 5m/hourly rollups preserved
    // Query availability using tier-select: should use hourly, get correct result
    todo!("Implement with test database");
}

#[tokio::test]
#[ignore]
async fn test_tier_select_fallback() {
    // Setup: create pool with partial tier data (e.g., only daily available)
    // Query 30-day range: QueryTier::select returns Rollups5m, but none exist
    // tier_select::query_with_fallback should fall back to daily
    // Assert correct results returned
    todo!("Implement with test database");
}
