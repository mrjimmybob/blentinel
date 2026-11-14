//! What is wrong now, across the user's companies (M6-03, D-114).

use chrono::{DateTime, Utc};
use sqlx::{Row, SqlitePool};

use super::checks::{parse_spec, stored_status};
use super::scope;
use crate::model::CompanyScope;
use crate::storage::{ProblemCheck, StoredProblem};

/// Where a probe is: its company (from its heartbeat, as the dashboard has
/// it) and site, its name, its last report. Archived companies and revoked
/// probes are left out.
const PLACE: &str = "
    FROM probe_heartbeats hb
    LEFT JOIN companies c ON c.id = hb.company_id
    LEFT JOIN probes p    ON p.id = hb.probe_id
    LEFT JOIN sites s     ON s.company_id = hb.company_id AND s.id = p.site_id";

const PLACE_COLUMNS: &str = "hb.company_id, COALESCE(c.name, hb.company_id) AS company_name,
    COALESCE(p.site_id, '') AS site_id, COALESCE(s.name, p.site_id, '') AS site_name,
    hb.probe_id, COALESCE(p.name, hb.probe_id) AS probe_name, hb.last_seen_at";

const KEPT: &str = "c.archived_at IS NULL AND COALESCE(p.state, 'active') <> 'revoked'";

fn place(row: &sqlx::sqlite::SqliteRow, check: Option<ProblemCheck>) -> StoredProblem {
    StoredProblem {
        company_id: row.get("company_id"),
        company_name: row.get("company_name"),
        site_id: row.get("site_id"),
        site_name: row.get("site_name"),
        probe_id: row.get("probe_id"),
        probe_name: row.get("probe_name"),
        last_seen_at: row.get::<Option<DateTime<Utc>>, _>("last_seen_at"),
        check,
    }
}

pub(super) async fn current(
    pool: &SqlitePool,
    user_scope: &CompanyScope,
) -> anyhow::Result<Vec<StoredProblem>> {
    let (in_scope, binds) = scope::condition("hb.company_id", user_scope);

    // Checks of probes still reporting whose latest result isn't Up.
    // Disabled checks aren't run any more; a deleted one has no row in
    // `checks`.
    let sql = format!(
        "WITH latest AS (SELECT probe_id, MAX(id) AS report_id FROM reports GROUP BY probe_id)
         SELECT {PLACE_COLUMNS},
                k.id AS check_id, k.name AS check_name, k.spec,
                cr.status, cr.reason, cr.message, cr.maintenance
         {PLACE}
         JOIN latest l         ON l.probe_id = hb.probe_id
         JOIN check_results cr ON cr.report_id = l.report_id
         JOIN checks k         ON k.id = cr.check_id AND k.enabled = 1
         WHERE hb.status = 'active' AND cr.status <> 'Up' AND {KEPT} AND {in_scope}"
    );
    let mut query = sqlx::query(&sql);
    for id in &binds {
        query = query.bind(id);
    }
    let mut problems = Vec::new();
    for row in query.fetch_all(pool).await? {
        let check = ProblemCheck {
            id: row.get("check_id"),
            name: row.get("check_name"),
            spec: parse_spec(row.get("spec"))?,
            status: stored_status(row.get("status"))?,
            reason: row.get("reason"),
            message: row.get("message"),
            in_maintenance: row.get("maintenance"),
        };
        problems.push(place(&row, Some(check)));
    }

    // Probes that stopped reporting: nobody knows how their checks are.
    let sql = format!(
        "SELECT {PLACE_COLUMNS} {PLACE}
         WHERE hb.status = 'expired' AND {KEPT} AND {in_scope}"
    );
    let mut query = sqlx::query(&sql);
    for id in &binds {
        query = query.bind(id);
    }
    for row in query.fetch_all(pool).await? {
        problems.push(place(&row, None));
    }
    Ok(problems)
}
