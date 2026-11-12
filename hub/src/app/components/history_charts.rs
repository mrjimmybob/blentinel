//! The check page's charts (M6-02): the status band with incidents and
//! maintenance, and one chart per metric. Built as SVG text, like the uptime
//! chart; every text that came from a probe is escaped.

use leptos::prelude::*;

use crate::model::{CheckHistory, MetricSeries, Status};

const WIDTH: f64 = 960.0;

/// Text for SVG: `&`, `<`, `>` and quotes escaped.
pub(crate) fn esc(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

/// A value for people: 0 to 2 decimals, as much as it needs.
pub(crate) fn number(value: f64) -> String {
    let text = if value.abs() >= 100.0 {
        format!("{value:.0}")
    } else if value.abs() >= 10.0 {
        format!("{value:.1}")
    } else {
        format!("{value:.2}")
    };
    if text.contains('.') {
        text.trim_end_matches('0').trim_end_matches('.').to_string()
    } else {
        text
    }
}

/// Where a moment falls across `width` pixels of `[from, to)`.
fn place(at: i64, from: i64, to: i64, width: f64) -> f64 {
    let span = (to - from).max(1) as f64;
    ((at - from).clamp(0, to - from) as f64 / span) * width
}

fn severity_class(status: Status) -> &'static str {
    crate::app::ui::status_class(status)
}

/// The band: a row of incidents, the status of each bucket (its worst), and
/// a row marking when results came during maintenance. Gaps had no results.
pub(crate) fn band_svg(h: &CheckHistory) -> String {
    const INCIDENTS_Y: f64 = 0.0;
    const BAND_Y: f64 = 12.0;
    const BAND_H: f64 = 28.0;
    const MAINT_Y: f64 = 42.0;
    const AXIS_Y: f64 = 62.0;
    let x = |at: i64| place(at, h.from_unix, h.to_unix, WIDTH);
    let mut svg = String::new();

    for b in &h.buckets {
        let (x0, x1) = (x(b.start_unix), x(b.start_unix + h.bucket_secs));
        let w = (x1 - x0).max(1.0);
        let c = &b.counts;
        let tip = format!(
            "{} UTC: Up {}, Warning {}, Down {}, Unknown {}, in maintenance {}",
            b.start, c.up, c.warning, c.down, c.unknown, c.maintenance
        );
        if let Some(shade) = c.worst() {
            svg.push_str(&format!(
                r#"<rect class="band {}" x="{x0:.1}" y="{BAND_Y}" width="{w:.1}" height="{BAND_H}"><title>{}</title></rect>"#,
                shade.css_class(),
                esc(&tip)
            ));
        }
        if c.maintenance > 0 {
            svg.push_str(&format!(
                r#"<rect class="band-maintenance" x="{x0:.1}" y="{MAINT_Y}" width="{w:.1}" height="6"><title>{}</title></rect>"#,
                esc(&tip)
            ));
        }
    }

    for i in &h.incidents {
        let x0 = x(i.opened_unix);
        let x1 = x(i.closed_unix.unwrap_or(h.to_unix));
        let w = (x1 - x0).max(3.0);
        let tip = format!(
            "Incident #{}: {} at worst, opened {} UTC, {}",
            i.id,
            i.severity.as_str(),
            i.opened_at,
            i.closed_at
                .as_ref()
                .map_or_else(|| "still open".to_string(), |c| format!("closed {c} UTC"))
        );
        svg.push_str(&format!(
            r#"<a href="/incidents/{}"><rect class="band-incident {}" x="{x0:.1}" y="{INCIDENTS_Y}" width="{w:.1}" height="8"><title>{}</title></rect></a>"#,
            i.id,
            severity_class(i.severity),
            esc(&tip)
        ));
    }

    svg.push_str(&format!(
        r#"<text class="axis-label" x="0" y="{AXIS_Y}" text-anchor="start">{}</text><text class="axis-label" x="{WIDTH}" y="{AXIS_Y}" text-anchor="end">{}</text>"#,
        esc(&h.from),
        esc(&h.to)
    ));
    format!(
        r#"<svg class="history-band" viewBox="0 0 {WIDTH} 66" role="img" aria-label="Status over the period">{svg}</svg>"#
    )
}

/// One metric: the average as a line, the lowest to highest as a shaded
/// band, broken where buckets have no value. Each point says its values on
/// hover.
pub(crate) fn metric_svg(series: &MetricSeries, h: &CheckHistory) -> String {
    const LEFT: f64 = 56.0;
    const TOP: f64 = 8.0;
    const PLOT_W: f64 = WIDTH - LEFT - 8.0;
    const PLOT_H: f64 = 150.0;
    const AXIS_Y: f64 = TOP + PLOT_H + 18.0;

    let points = &series.points;
    let lowest = points.iter().map(|p| p.min).fold(f64::INFINITY, f64::min);
    let highest = points
        .iter()
        .map(|p| p.max)
        .fold(f64::NEG_INFINITY, f64::max);
    // From 0 when every value is positive, so small changes don't look big.
    let lo = lowest.min(0.0);
    let hi = if highest > lo { highest } else { lo + 1.0 };
    let y = |v: f64| TOP + PLOT_H - (v - lo) / (hi - lo) * PLOT_H;
    let x = |at: i64| LEFT + place(at + h.bucket_secs / 2, h.from_unix, h.to_unix, PLOT_W);
    let unit = esc(&series.unit);

    let mut svg = String::new();
    for (value, anchor) in [
        (hi, TOP),
        (lo + (hi - lo) / 2.0, TOP + PLOT_H / 2.0),
        (lo, TOP + PLOT_H),
    ] {
        svg.push_str(&format!(
            r#"<line class="gridline" x1="{LEFT}" y1="{anchor:.1}" x2="{:.1}" y2="{anchor:.1}"/><text class="axis-label" x="{:.1}" y="{:.1}" text-anchor="end">{}{unit}</text>"#,
            LEFT + PLOT_W,
            LEFT - 6.0,
            anchor + 4.0,
            number(value)
        ));
    }

    // Runs of points without a gap longer than one bucket.
    let mut runs: Vec<&[crate::model::SeriesPoint]> = Vec::new();
    let mut start = 0;
    for i in 1..=points.len() {
        let gap =
            i == points.len() || points[i].start_unix - points[i - 1].start_unix > h.bucket_secs;
        if gap {
            runs.push(&points[start..i]);
            start = i;
        }
    }
    for run in runs.iter().filter(|r| !r.is_empty()) {
        let upper = run
            .iter()
            .map(|p| format!("{:.1},{:.1}", x(p.start_unix), y(p.max)));
        let lower = run
            .iter()
            .rev()
            .map(|p| format!("{:.1},{:.1}", x(p.start_unix), y(p.min)));
        let band: Vec<String> = upper.chain(lower).collect();
        svg.push_str(&format!(
            r#"<polygon class="metric-range" points="{}"/>"#,
            band.join(" ")
        ));
        let line: Vec<String> = run
            .iter()
            .map(|p| format!("{:.1},{:.1}", x(p.start_unix), y(p.avg)))
            .collect();
        svg.push_str(&format!(
            r#"<polyline class="metric-avg" points="{}"/>"#,
            line.join(" ")
        ));
    }
    for p in points {
        let tip = format!(
            "{} UTC: average {}{u}, lowest {}{u}, highest {}{u}",
            p.start,
            number(p.avg),
            number(p.min),
            number(p.max),
            u = series.unit
        );
        svg.push_str(&format!(
            r#"<circle class="metric-point" cx="{:.1}" cy="{:.1}" r="4"><title>{}</title></circle>"#,
            x(p.start_unix),
            y(p.avg),
            esc(&tip)
        ));
    }
    svg.push_str(&format!(
        r#"<text class="axis-label" x="{LEFT}" y="{AXIS_Y:.1}" text-anchor="start">{}</text><text class="axis-label" x="{:.1}" y="{AXIS_Y:.1}" text-anchor="end">{}</text>"#,
        esc(&h.from),
        LEFT + PLOT_W,
        esc(&h.to)
    ));
    format!(
        r#"<svg class="metric-chart" viewBox="0 0 {WIDTH} {:.0}" role="img" aria-label="{}">{svg}</svg>"#,
        AXIS_Y + 6.0,
        esc(&series.key)
    )
}

/// The status band, as a component.
#[component]
pub(crate) fn StatusBand(history: CheckHistory) -> impl IntoView {
    let svg = band_svg(&history);
    view! {
        <div class="history-band-wrap" inner_html=svg></div>
        <div class="history-legend">
            <span class="legend up">"Up"</span>
            <span class="legend warning">"Warning"</span>
            <span class="legend down">"Down"</span>
            <span class="legend unknown">"Unknown"</span>
            <span class="legend maintenance">"In maintenance"</span>
            <span class="legend incident">"Incident (top row; click to open)"</span>
        </div>
    }
}

/// One metric's chart, as a component.
#[component]
pub(crate) fn MetricChart(series: MetricSeries, history: CheckHistory) -> impl IntoView {
    let svg = metric_svg(&series, &history);
    view! { <div class="metric-chart-wrap" inner_html=svg></div> }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{IncidentSpan, SeriesPoint, StatusBucket, StatusCounts};

    fn history() -> CheckHistory {
        CheckHistory {
            check_id: "c".into(),
            name: "c".into(),
            kind: "ping".into(),
            target: "t".into(),
            enabled: true,
            probe_id: "p".into(),
            probe_name: "p".into(),
            company_id: "acme".into(),
            company_name: "Acme".into(),
            site_name: "HQ".into(),
            from: "2026-03-10 00:00".into(),
            to: "2026-03-10 01:00".into(),
            from_unix: 0,
            to_unix: 3600,
            bucket_secs: 300,
            totals: StatusCounts::default(),
            availability_pct: None,
            incident_count: 0,
            mttr_secs: None,
            buckets: vec![],
            series: vec![],
            incidents: vec![],
            recent: vec![],
        }
    }

    #[test]
    fn text_from_a_probe_is_escaped() {
        assert_eq!(
            esc(r#"<script>"a" & 'b'</script>"#),
            "&lt;script&gt;&quot;a&quot; &amp; &#39;b&#39;&lt;/script&gt;"
        );
        let series = MetricSeries {
            key: "<b>".into(),
            unit: "<i>".into(),
            points: vec![],
        };
        let svg = metric_svg(&series, &history());
        assert!(!svg.contains("<b>") && !svg.contains("<i>"), "{svg}");
    }

    #[test]
    fn numbers_keep_only_the_decimals_they_need() {
        assert_eq!(number(12.0), "12");
        assert_eq!(number(12.34), "12.3");
        assert_eq!(number(0.126), "0.13");
        assert_eq!(number(1234.5), "1234");
        assert_eq!(number(0.0), "0");
    }

    #[test]
    fn buckets_and_incidents_sit_where_their_time_falls() {
        let mut h = history();
        h.buckets = vec![StatusBucket {
            start_unix: 1800,
            start: "2026-03-10 00:30".into(),
            counts: StatusCounts {
                down: 1,
                maintenance: 1,
                ..StatusCounts::default()
            },
        }];
        h.incidents = vec![IncidentSpan {
            id: 7,
            severity: Status::Down,
            opened_at: "2026-03-10 00:30".into(),
            opened_unix: 1800,
            closed_at: None,
            closed_unix: None,
        }];
        let svg = band_svg(&h);
        // Half way along, one 5-minute bucket wide (960 × 300 / 3600 = 80).
        assert!(
            svg.contains(r#"class="band down" x="480.0" y="12" width="80.0""#),
            "{svg}"
        );
        assert!(
            svg.contains(r#"class="band-maintenance" x="480.0""#),
            "{svg}"
        );
        // Still open: to the end.
        assert!(svg.contains(r#"<a href="/incidents/7"><rect class="band-incident down" x="480.0" y="0" width="480.0""#), "{svg}");
    }

    #[test]
    fn a_metric_line_breaks_where_buckets_have_no_value() {
        let point = |start_unix| SeriesPoint {
            start_unix,
            start: String::new(),
            min: 1.0,
            max: 3.0,
            avg: 2.0,
        };
        let series = MetricSeries {
            key: "cpu".into(),
            unit: "%".into(),
            points: vec![point(0), point(300), point(1800)],
        };
        let svg = metric_svg(&series, &history());
        assert_eq!(svg.matches("<polyline").count(), 2, "{svg}");
        assert_eq!(svg.matches("<circle").count(), 3);
    }
}
