//! Rotations of the hub's certificate and key, and the pins probes hold
//! (M2-13, D-61).

use chrono::{DateTime, Utc};
use sqlx::{Row, SqlitePool};

use crate::model::RotationState;
use crate::storage::{NewRotation, ProbePins, RotationRecord};

pub(super) async fn rotations(
    pool: &SqlitePool,
    limit: u32,
) -> anyhow::Result<Vec<RotationRecord>> {
    let rows = sqlx::query(
        "SELECT id, rotates_tls, rotates_key, window_days, started_by, started_at, deadline_at,
                state, ended_by, ended_at, deadline_warned
         FROM hub_rotations ORDER BY id DESC LIMIT ?",
    )
    .bind(i64::from(limit))
    .fetch_all(pool)
    .await?;
    rows.into_iter()
        .map(|row| {
            let state: String = row.get("state");
            Ok(RotationRecord {
                id: row.get("id"),
                rotates_tls: row.get("rotates_tls"),
                rotates_key: row.get("rotates_key"),
                window_days: u32::try_from(row.get::<i64, _>("window_days"))?,
                started_by: row.get("started_by"),
                started_at: row.get("started_at"),
                deadline_at: row.get("deadline_at"),
                state: RotationState::parse(&state)
                    .ok_or_else(|| anyhow::anyhow!("unknown rotation state {state}"))?,
                ended_by: row.get("ended_by"),
                ended_at: row.get("ended_at"),
                deadline_warned: row.get("deadline_warned"),
            })
        })
        .collect()
}

pub(super) async fn start(pool: &SqlitePool, rotation: &NewRotation) -> anyhow::Result<i64> {
    Ok(sqlx::query_scalar(
        "INSERT INTO hub_rotations
             (rotates_tls, rotates_key, window_days, started_by, started_at, deadline_at, state)
         VALUES (?, ?, ?, ?, ?, ?, 'announcing') RETURNING id",
    )
    .bind(rotation.rotates_tls)
    .bind(rotation.rotates_key)
    .bind(i64::from(rotation.window_days))
    .bind(&rotation.started_by)
    .bind(rotation.started_at)
    .bind(rotation.deadline_at)
    .fetch_one(pool)
    .await?)
}

pub(super) async fn end(
    pool: &SqlitePool,
    id: i64,
    state: RotationState,
    ended_by: Option<&str>,
    at: DateTime<Utc>,
) -> anyhow::Result<bool> {
    let done = sqlx::query(
        "UPDATE hub_rotations SET state = ?, ended_by = ?, ended_at = ?
         WHERE id = ? AND state = 'announcing'",
    )
    .bind(state.as_str())
    .bind(ended_by)
    .bind(at)
    .bind(id)
    .execute(pool)
    .await?;
    Ok(done.rows_affected() > 0)
}

pub(super) async fn mark_warned(pool: &SqlitePool, id: i64) -> anyhow::Result<()> {
    sqlx::query("UPDATE hub_rotations SET deadline_warned = 1 WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

pub(super) async fn active_probe_pins(pool: &SqlitePool) -> anyhow::Result<Vec<ProbePins>> {
    let rows = sqlx::query(
        "SELECT id, pins_current, pins_next FROM probes WHERE state = 'active' ORDER BY id",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| ProbePins {
            probe_id: row.get("id"),
            current: row.get("pins_current"),
            next: row.get("pins_next"),
        })
        .collect())
}
