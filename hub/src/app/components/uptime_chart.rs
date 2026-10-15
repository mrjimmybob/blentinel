//! Uptime chart (SVG) for a company.

use leptos::prelude::*;

use crate::model::UptimeBucket;

#[component]
pub(crate) fn UptimeChart(
    buckets: Vec<UptimeBucket>,
    title: Signal<String>,
    selected_range: RwSignal<String>,
) -> impl IntoView {
    const MARGIN_L: f64 = 60.0;
    const MARGIN_R: f64 = 20.0;
    const MARGIN_T: f64 = 20.0;
    const MARGIN_B: f64 = 40.0;
    const PLOT_W: f64 = 960.0 - MARGIN_L - MARGIN_R; // 880
    const PLOT_H: f64 = 300.0 - MARGIN_T - MARGIN_B; // 240

    // Build the entire SVG content as a string, then render via a single <div> wrapper
    let svg_inner: String = if buckets.is_empty() {
        r#"<text class="no-data" x="480" y="150" text-anchor="middle">No uptime data available</text>"#.to_string()
    } else {
        let bucket_count = buckets.len();

        // Compute percentage per bucket
        let points: Vec<(f64, f64)> = buckets
            .iter()
            .enumerate()
            .map(|(i, b)| {
                let total = b.up_count + b.down_count;
                let pct = if total == 0 {
                    100.0
                } else {
                    (b.up_count as f64 / total as f64) * 100.0
                };
                let x = MARGIN_L + (i as f64 / (bucket_count - 1).max(1) as f64) * PLOT_W;
                let y = MARGIN_T + PLOT_H - (pct / 100.0) * PLOT_H;
                (x, y)
            })
            .collect();

        // Y-axis gridlines
        let mut svg = String::new();
        let y_ticks: [(f64, &str); 5] = [
            (0.0, "100%"),
            (25.0, "75%"),
            (50.0, "50%"),
            (75.0, "25%"),
            (100.0, "0%"),
        ];
        for (pct, label) in &y_ticks {
            let y = MARGIN_T + (pct / 100.0) * PLOT_H;
            svg.push_str(&format!(
                r#"<line class="gridline" x1="{}" y1="{:.1}" x2="{}" y2="{:.1}"/>"#,
                MARGIN_L,
                y,
                MARGIN_L + PLOT_W,
                y
            ));
            svg.push_str(&format!(
                r#"<text class="axis-label" x="{}" y="{:.1}" text-anchor="end">{}</text>"#,
                MARGIN_L - 8.0,
                y + 4.0,
                label
            ));
        }

        // X-axis labels — format and density depend on selected range.
        // Target ~8 labels regardless of bucket count.
        let range = selected_range.get_untracked();
        let step = (bucket_count / 8).max(1);
        for (i, b) in buckets.iter().enumerate() {
            if i % step != 0 {
                continue;
            }
            let x = MARGIN_L + (i as f64 / (bucket_count - 1).max(1) as f64) * PLOT_W;
            let y = 300.0 - MARGIN_B + 18.0;
            let s = &b.bucket;
            let label: String = match range.as_str() {
                "24h" => {
                    if s.len() >= 16 {
                        s[11..16].to_string()
                    } else {
                        s.clone()
                    }
                }
                // DD/MM (European format): chars 8-10 = day, 5-7 = month
                _ => {
                    if s.len() >= 10 {
                        format!("{}/{}", &s[8..10], &s[5..7])
                    } else {
                        s.clone()
                    }
                }
            };
            svg.push_str(&format!(
                r#"<text class="axis-label" x="{:.1}" y="{:.1}" text-anchor="middle">{}</text>"#,
                x, y, label
            ));
        }

        // Gradient definition
        svg.push_str(
            r#"<defs>
            <linearGradient id="uptime-gradient" x1="0" y1="0" x2="0" y2="1">
                <stop offset="0%" stop-color="var(--color-up)" stop-opacity="0.3"/>
                <stop offset="100%" stop-color="var(--color-up)" stop-opacity="0.0"/>
            </linearGradient>
        </defs>"#,
        );

        // Maintenance windows (D-65): their results don't count; shade the buckets.
        let step_w = PLOT_W / (bucket_count - 1).max(1) as f64;
        for (b, (x, _)) in buckets.iter().zip(&points) {
            if b.maintenance_count == 0 {
                continue;
            }
            let left = (x - step_w / 2.0).max(MARGIN_L);
            let right = (x + step_w / 2.0).min(MARGIN_L + PLOT_W);
            svg.push_str(&format!(
                r#"<rect class="maintenance-band" x="{:.1}" y="{:.1}" width="{:.1}" height="{:.1}"><title>Maintenance</title></rect>"#,
                left,
                MARGIN_T,
                (right - left).max(1.0),
                PLOT_H
            ));
        }

        // Area path
        if let (Some(&(first_x, first_y)), Some(&(last_x, _))) = (points.first(), points.last()) {
            let mut d = format!("M {:.1},{:.1}", first_x, first_y);
            for (x, y) in &points[1..] {
                d.push_str(&format!(" L {:.1},{:.1}", x, y));
            }
            d.push_str(&format!(
                " L {:.1},{:.1} L {:.1},{:.1} Z",
                last_x,
                MARGIN_T + PLOT_H,
                first_x,
                MARGIN_T + PLOT_H
            ));
            svg.push_str(&format!(r#"<path class="area-fill" d="{}"/>"#, d));
        }

        // Line
        let line_points: String = points
            .iter()
            .map(|(x, y)| format!("{:.1},{:.1}", x, y))
            .collect::<Vec<_>>()
            .join(" ");
        svg.push_str(&format!(
            r#"<polyline class="line" points="{}"/>"#,
            line_points
        ));

        // Data points
        for (x, y) in &points {
            svg.push_str(&format!(
                r#"<circle class="point" cx="{:.1}" cy="{:.1}" r="3"/>"#,
                x, y
            ));
        }

        svg
    };

    let _full_svg = format!(
        r#"<svg class="chart-svg" viewBox="0 0 960 300">{}</svg>"#,
        svg_inner
    );

    // We render the chart section header + SVG. The SVG is injected as raw HTML
    // via an Effect that sets innerHTML on a container div.
    let chart_ref = NodeRef::<leptos::html::Div>::new();

    Effect::new(move |_| {
        #[cfg(not(feature = "ssr"))]
        {
            if let Some(el) = chart_ref.get() {
                el.set_inner_html(&_full_svg);
            }
        }
    });

    let ranges: Vec<(&'static str, &'static str)> =
        vec![("24h", "24h"), ("7d", "7d"), ("30d", "30d"), ("all", "All")];

    view! {
        <div class="chart-container">
            <div class="section-header">
                <h2>{move || title.get()}</h2>
                <div class="range-pills">
                    {ranges.into_iter().map(|(value, label)| {
                        let value_owned = value.to_string();
                        let value_for_click = value.to_string();
                        view! {
                            <button
                                class=move || {
                                    if selected_range.get() == value_owned {
                                        "range-pill range-pill-active"
                                    } else {
                                        "range-pill"
                                    }
                                }
                                on:click=move |_| {
                                    selected_range.set(value_for_click.clone());
                                }
                            >
                                {label}
                            </button>
                        }
                    }).collect::<Vec<_>>()}
                </div>
            </div>
            <div node_ref=chart_ref></div>
        </div>
    }
}
