//! Small presentation helpers shared by the pages and components.

use leptos::prelude::*;

use crate::model::Severity;

/// Type-erase a view so different branches can be returned from the same closure.
pub(crate) fn any(v: impl IntoView) -> AnyView {
    v.into_any()
}

/// An empty view, for the branch of a conditional that renders nothing.
pub(crate) fn nothing() -> AnyView {
    ().into_any()
}

/// Badge text for a severity. The rule itself lives in `crate::model::Severity`
/// and is evaluated by the server; the UI only presents it.
pub(crate) fn severity_label(severity: Severity) -> &'static str {
    match severity {
        Severity::Critical => "CRITICAL",
        Severity::Degraded => "DEGRADED",
        Severity::Healthy => "HEALTHY",
    }
}

/// Company-card border class for a severity.
pub(crate) fn severity_border_class(severity: Severity) -> &'static str {
    match severity {
        Severity::Critical => "card-border-red",
        Severity::Degraded => "card-border-amber",
        Severity::Healthy => "card-border-green",
    }
}

/// The value of the `<input>` or `<select>` an event came from ("" otherwise).
pub(crate) fn input_value(ev: &leptos::ev::Event) -> String {
    use leptos::wasm_bindgen::JsCast;
    let Some(target) = ev.target() else {
        return String::new();
    };
    if let Some(input) = target.dyn_ref::<leptos::web_sys::HtmlInputElement>() {
        input.value()
    } else if let Some(select) = target.dyn_ref::<leptos::web_sys::HtmlSelectElement>() {
        select.value()
    } else {
        String::new()
    }
}

/// Format an ISO 8601 timestamp string (e.g. "2026-03-08T01:27:26.522816772Z")
/// into "YYYY-MM-DD hh:mm:ss.sss" for display.
pub(crate) fn fmt_ts(ts: &str) -> String {
    let s = ts.trim_end_matches('Z');
    if let Some((date, time)) = s.split_once('T') {
        let time_fmt = if let Some((secs, frac)) = time.split_once('.') {
            let ms: String = frac.chars().take(3).collect();
            format!("{}.{}", secs, ms)
        } else {
            time.to_string()
        };
        format!("{} {}", date, time_fmt)
    } else {
        ts.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The severity *rule* is tested in crate::model; these pin its presentation.

    #[test]
    fn each_severity_has_its_own_label_and_border() {
        let all = [Severity::Critical, Severity::Degraded, Severity::Healthy];
        assert_eq!(all.map(severity_label), ["CRITICAL", "DEGRADED", "HEALTHY"]);
        assert_eq!(
            all.map(severity_border_class),
            ["card-border-red", "card-border-amber", "card-border-green"]
        );
    }

    #[test]
    fn fmt_ts_trims_to_milliseconds_and_drops_the_t_and_z() {
        assert_eq!(
            fmt_ts("2026-03-08T01:27:26.522816772Z"),
            "2026-03-08 01:27:26.522"
        );
        assert_eq!(fmt_ts("2026-03-08T01:27:26Z"), "2026-03-08 01:27:26");
    }

    #[test]
    fn fmt_ts_passes_through_unrecognised_input() {
        assert_eq!(fmt_ts("not a timestamp"), "not a timestamp");
    }
}
