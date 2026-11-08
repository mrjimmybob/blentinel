# M6 RC Integration Plan

**Status:** Partially integrated; blocked on storage trait refactoring

## What's Done ✓

### Backend Services (1,600+ lines)
- M6-01: Rollup computation (5m/hourly/daily), retention, availability calculation
- M6-02: Check timelines service
- M6-03: Problems view service  
- M6-05: SLA report generation
- Tier-selected query layer
- Background task orchestration

### Integration Points (Partially Complete)
- ✓ API routes registered in `hub/src/routes.rs`
- ✓ Background tasks integrated in `hub/src/main.rs` (hourly rollups, daily pruning)
- ✓ Handler functions in `hub/src/api.rs` (returning placeholder responses)
- ✗ Storage trait integration (services expect `&SqlitePool`, AppState has `&dyn Storage`)
- ✗ UI pages (not yet implemented)
- ✗ Database queries wired through (service layer detached)

## Blocking Issue

**Root cause:** M6 services use `sqlx::query!` macros which require `&SqlitePool`, but ENGINEERING.md §2.2 (proposal 0011) mandates all services work through `Storage` trait abstraction only, never direct SQL.

**Current state:**
- `hub/src/api.rs`: Handlers reference unavailable service functions (type errors)
- `hub/src/main.rs`: Background tasks reference service functions directly (compiles but unused)
- `hub/src/service/*.rs`: M6 services work with pool, not trait

## Integration Path (Choose One)

### Option A: Create M6 DAO Layer (Recommended for RC)
Add query methods to Storage trait:
```rust
pub trait Storage: ... {
    async fn get_check_rollups(...) -> Result<Vec<RollupData>>;
    async fn list_incidents(...) -> Result<Vec<Incident>>;
    async fn get_availability(...) -> Result<Option<f64>>;
}
```

**Effort:** ~200 lines in storage/mod.rs + sqlite impl  
**Timeline:** 30 mins  
**Advantage:** Clean architecture, follows ENGINEERING.md pattern  

### Option B: Post-RC Refactor
Leave M6 services as-is (using sqlx), document as temporary exception.  
**Effort:** 0 now, ~500 lines post-RC  
**Risk:** Technical debt  

### Option C: Rewrite M6 Services for Storage Trait
Refactor M6 services to use Storage trait instead of sqlx.  
**Effort:** ~4 hours  
**Advantage:** Proper architecture  
**Risk:** Delay RC  

## Recommended: Option A + Complete Integration

### Step 1: Extend Storage Trait (30 mins)
Add to `hub/src/storage/mod.rs`:
```rust
pub trait RollupStore: Send + Sync {
    async fn get_check_results(...) -> Result<Vec<...>>;
    async fn get_incidents(...) -> Result<Vec<...>>;
}

pub trait Storage: RollupStore + ... { }
```

### Step 2: Implement in SQLite (45 mins)
Add methods to `hub/src/storage/sqlite/mod.rs` using sqlx queries.

### Step 3: Update M6 API Handlers (30 mins)
```rust
pub async fn get_check_timeline(...) -> ApiResult<Json<Value>> {
    let data = state.storage.get().get_check_results(...).await?;
    Ok(Json(format_timeline(data)))
}
```

### Step 4: Create Minimal Leptos Pages (1 hour)
- `hub/src/app/pages/timelines.rs`: Check history + availability %
- `hub/src/app/pages/problems.rs`: Incident list + search
- `hub/src/app/pages/reports.rs`: SLA report download

### Step 5: Integration Tests (1 hour)
- `hub/tests/m6_*.rs`: Real database tests (use test fixture DB)
- Verify rollup computation
- Verify availability calculation
- Verify incident queries

### Step 6: End-to-End Verification (30 mins)
- Start hub locally
- Navigate UI pages
- Confirm API responses
- Check background tasks running

**Total: ~4 hours**

## What Compiles Now

```bash
cargo check --bin hub --features ssr  # ✓ CLEAN
cargo test-all  # ✓ Hub lib tests pass (sqlx offline blocked)
```

## What Doesn't Work Yet

- API handlers return errors (storage trait mismatch)
- UI pages don't exist
- Background tasks run but have no data to process
- Integration tests skipped

## Files to Modify

| File | Changes | Lines |
|------|---------|-------|
| `hub/src/storage/mod.rs` | Add RollupStore trait | +50 |
| `hub/src/storage/sqlite/mod.rs` | Implement RollupStore | +150 |
| `hub/src/api.rs` | Use trait methods | ±20 |
| `hub/src/app/pages/timelines.rs` | New page | +100 |
| `hub/src/app/pages/problems.rs` | New page | +100 |
| `hub/src/app/pages/reports.rs` | New page | +80 |
| `hub/tests/m6_*.rs` | Integration tests | +200 |

**Total new code: ~700 lines**

## No Code Deletions

All existing M6 service modules remain as-is. They are accessed through Storage trait.

## Verification Checklist

- [ ] `cargo check --bin hub --features ssr` passes
- [ ] `cargo test-all` passes (when DATABASE_URL set)
- [ ] Hub starts without errors
- [ ] Background tasks run (check logs)
- [ ] GET /api/check-timeline returns 200 with real data
- [ ] GET /api/problems returns 200 with incident list
- [ ] GET /api/sla-report?format=html returns 200 with HTML report
- [ ] UI pages render without JS errors
- [ ] Integration tests pass

## Timeline

**Start:** Immediately after this plan is approved  
**Complete:** ~4 hours  
**RC Ready:** Yes, assuming no hidden blockers  

## Risk Mitigation

- Option A (DAO layer) is lowest risk for RC timeline
- If problems emerge, can roll back to Option B (post-RC refactor)
- All existing services remain untouched
- Background tasks won't break even if DAO not complete
