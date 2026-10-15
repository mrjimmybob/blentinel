//! The `eventlog` check (M5-14, D-91): the events of a Windows event log
//! that a source, event IDs and a level pick out, in the last minutes. How
//! many there are, and the latest one in words.
//!
//! Disk errors (`System`, source `disk`, IDs 7, 51, 153), a backup that
//! failed, or the other way round an event that must be there: last
//! night's "backup succeeded". The probe counts and reports; the hub says
//! whether that is trouble (`service::thresholds::eventlog_verdict`).
//!
//! Windows' own query API, natively (FP-2): no `wevtutil`, no PowerShell.
//! The probe reads; it writes no event and clears no log.

use common::checks::EventLevel;
#[cfg(any(windows, test))]
use common::checks::{MAX_EVENT_COUNT, Metric, metric};

use super::Outcome;

/// What picks the events out.
#[derive(Debug, Clone, PartialEq)]
pub struct Query<'a> {
    pub log: &'a str,
    pub source: &'a str,
    pub ids: &'a [u32],
    pub level: EventLevel,
    pub minutes: u32,
}

/// The latest event found.
#[cfg(any(windows, test))]
#[derive(Debug, Clone, Default, PartialEq)]
struct Event {
    id: u32,
    source: String,
    /// When it was written, UTC: `2026-10-12 14:02`.
    time: String,
    /// Its message, where Windows can put it in words.
    text: String,
}

/// The longest message of an event shown, in characters.
#[cfg(any(windows, test))]
const MAX_TEXT_CHARS: usize = 200;

/// The query, in the XPath Windows' event logs take. The names in it were
/// checked when the check was saved (`common`: no quote can be in them).
#[cfg(any(windows, test))]
fn xpath(query: &Query) -> String {
    let mut wanted = Vec::new();
    if !query.source.is_empty() {
        wanted.push(format!("Provider[@Name='{}']", query.source));
    }
    if !query.ids.is_empty() {
        let ids: Vec<String> = query.ids.iter().map(|id| format!("EventID={id}")).collect();
        wanted.push(format!("({})", ids.join(" or ")));
    }
    // Windows' levels: 1 critical, 2 error, 3 warning, 4 information, and
    // 0 for events that carry none (audits: information).
    wanted.push(match query.level {
        EventLevel::Critical => "(Level=1)".to_string(),
        EventLevel::Error => "(Level=1 or Level=2)".to_string(),
        EventLevel::Warning => "(Level=1 or Level=2 or Level=3)".to_string(),
        EventLevel::Information => "(Level>=0 and Level<=4)".to_string(),
    });
    wanted.push(format!(
        "TimeCreated[timediff(@SystemTime) <= {}]",
        u64::from(query.minutes) * 60_000
    ));
    format!("*[System[{}]]", wanted.join(" and "))
}

/// The value of the attribute that `start` begins (`<Provider Name=`), in
/// either kind of quotes.
#[cfg(any(windows, test))]
fn attribute<'a>(xml: &'a str, start: &str) -> Option<&'a str> {
    let rest = &xml[xml.find(start)? + start.len()..];
    let quote = rest.chars().next().filter(|c| *c == '\'' || *c == '"')?;
    let value = &rest[1..];
    Some(&value[..value.find(quote)?])
}

/// The source, the ID and the time of an event from its XML, as Windows
/// renders it.
#[cfg(any(windows, test))]
fn from_xml(xml: &str) -> Option<Event> {
    let source = attribute(xml, "<Provider Name=")?;
    // `<EventID Qualifiers='49156'>51</EventID>`, or `<EventID>51</EventID>`.
    let id = &xml[xml.find("<EventID")?..];
    let id = &id[id.find('>')? + 1..];
    let id = &id[..id.find('<')?];
    let time = attribute(xml, "SystemTime=")?;
    Some(Event {
        id: id.trim().parse().ok()?,
        source: unescape(source),
        time: time.get(..16)?.replace('T', " "),
        text: String::new(),
    })
}

/// The few characters XML writes another way.
#[cfg(any(windows, test))]
fn unescape(text: &str) -> String {
    text.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&apos;", "'")
        .replace("&quot;", "\"")
}

/// An event's message for a line: its first sentence or so.
#[cfg(any(windows, test))]
fn one_line(text: &str) -> String {
    let line: String = text
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let line = line.split_whitespace().collect::<Vec<_>>().join(" ");
    match line.char_indices().nth(MAX_TEXT_CHARS) {
        Some((at, _)) => format!("{}…", &line[..at]),
        None => line,
    }
}

/// A length of time in words: `45 minutes`, `26 hours`, `7 days`.
#[cfg(any(windows, test))]
fn span(minutes: u32) -> String {
    let count = |n: u32, unit: &str| format!("{n} {unit}{}", if n == 1 { "" } else { "s" });
    match minutes {
        m if m % 1440 == 0 => count(m / 1440, "day"),
        m if m % 60 == 0 => count(m / 60, "hour"),
        m => count(m, "minute"),
    }
}

/// What was found, in words and as a number. Always up: whether events are
/// trouble is the hub's to say.
#[cfg(any(windows, test))]
fn describe(count: u32, latest: Option<&Event>, minutes: u32) -> Outcome {
    let within = format!("in the last {}", span(minutes));
    let message = match (count, latest) {
        (0, _) => format!("No such event {within}"),
        (count, latest) => {
            let many = match count {
                1 => "1 event".to_string(),
                n if n >= MAX_EVENT_COUNT => format!("{MAX_EVENT_COUNT} events or more"),
                n => format!("{n} events"),
            };
            match latest {
                Some(event) => {
                    let text = match event.text.as_str() {
                        "" => String::new(),
                        text => format!(": {text}"),
                    };
                    format!(
                        "{many} {within}; the latest: {} {} at {} UTC{text}",
                        event.source, event.id, event.time
                    )
                }
                None => format!("{many} {within}"),
            }
        }
    };
    Outcome::up(message).with_metrics(vec![Metric::new(metric::EVENTS, f64::from(count), "")])
}

pub async fn check(query: Query<'_>) -> Outcome {
    platform::check(query).await
}

#[cfg(windows)]
mod platform {
    use windows::Win32::Foundation::{
        ERROR_ACCESS_DENIED, ERROR_EVT_CHANNEL_NOT_FOUND, ERROR_EVT_INVALID_QUERY,
        ERROR_INSUFFICIENT_BUFFER,
    };
    use windows::Win32::System::EventLog::{
        EVT_HANDLE, EvtClose, EvtFormatMessage, EvtFormatMessageEvent, EvtNext,
        EvtOpenPublisherMetadata, EvtQuery, EvtQueryChannelPath, EvtQueryReverseDirection,
        EvtRender, EvtRenderEventXml,
    };
    use windows::core::{HSTRING, PCWSTR};

    use super::{Event, MAX_EVENT_COUNT, Outcome, Query, describe, from_xml, one_line, xpath};

    /// How many events are taken from the log at a time.
    const BATCH: usize = 64;
    /// How long Windows may take to hand over a batch, in milliseconds.
    const NEXT_TIMEOUT_MS: u32 = 5000;

    /// A handle of the event log API, closed when dropped.
    struct Handle(EVT_HANDLE);

    impl Drop for Handle {
        fn drop(&mut self) {
            // SAFETY: the handle came from the API and is closed once.
            unsafe {
                let _ = EvtClose(self.0);
            }
        }
    }

    pub async fn check(query: Query<'_>) -> Outcome {
        let (log, text, minutes) = (query.log.to_string(), xpath(&query), query.minutes);
        // The event log's calls block.
        tokio::task::spawn_blocking(move || match read(&log, &text) {
            Ok((count, latest)) => describe(count, latest.as_ref(), minutes),
            Err(e) => Outcome::down(failure(&log, &e)),
        })
        .await
        .unwrap_or_else(|_| Outcome::down("Reading the event log failed"))
    }

    fn failure(log: &str, error: &windows::core::Error) -> String {
        let code = error.code();
        if code == ERROR_EVT_CHANNEL_NOT_FOUND.to_hresult() {
            format!("No event log named {log} on this machine")
        } else if code == ERROR_ACCESS_DENIED.to_hresult() {
            format!("The probe may not read the {log} log")
        } else if code == ERROR_EVT_INVALID_QUERY.to_hresult() {
            format!("The {log} log doesn't take this check's query")
        } else {
            format!("Can't read the {log} log: {}", error.message())
        }
    }

    /// How many events of `log` match (counted up to [`MAX_EVENT_COUNT`]),
    /// and the newest of them.
    fn read(log: &str, query: &str) -> windows::core::Result<(u32, Option<Event>)> {
        let (log, query) = (HSTRING::from(log), HSTRING::from(query));
        // SAFETY: the texts live through the call; the handles it gives
        // are owned by `Handle`s and closed.
        let results = Handle(unsafe {
            EvtQuery(
                None,
                &log,
                &query,
                EvtQueryChannelPath.0 | EvtQueryReverseDirection.0,
            )?
        });
        let (mut count, mut latest) = (0u32, None);
        while count < MAX_EVENT_COUNT {
            let mut batch = [0isize; BATCH];
            let mut got = 0u32;
            // SAFETY: `batch` and `got` are valid for the call.
            let next = unsafe { EvtNext(results.0, &mut batch, NEXT_TIMEOUT_MS, 0, &mut got) };
            if next.is_err() || got == 0 {
                // No more items (or the log stopped answering): what was
                // counted stands.
                break;
            }
            let events: Vec<Handle> = batch[..got as usize]
                .iter()
                .map(|event| Handle(EVT_HANDLE(*event)))
                .collect();
            if latest.is_none() {
                // Newest first: the first of the first batch.
                latest = events.first().and_then(described);
            }
            count = count.saturating_add(got);
        }
        Ok((count.min(MAX_EVENT_COUNT), latest))
    }

    /// An event's source, ID, time and message.
    fn described(event: &Handle) -> Option<Event> {
        let mut found = from_xml(&xml(event)?)?;
        found.text = message(event, &found.source).unwrap_or_default();
        Some(found)
    }

    /// The event as XML.
    fn xml(event: &Handle) -> Option<String> {
        let (mut used, mut properties) = (0u32, 0u32);
        // SAFETY: first asked how many bytes it takes, then given a buffer
        // of that many.
        unsafe {
            let _ = EvtRender(
                None,
                event.0,
                EvtRenderEventXml.0,
                0,
                None,
                &mut used,
                &mut properties,
            );
            if used == 0 {
                return None;
            }
            let mut buffer = vec![0u16; (used as usize).div_ceil(2)];
            EvtRender(
                None,
                event.0,
                EvtRenderEventXml.0,
                (buffer.len() * 2) as u32,
                Some(buffer.as_mut_ptr().cast()),
                &mut used,
                &mut properties,
            )
            .ok()?;
            let end = buffer.iter().position(|c| *c == 0).unwrap_or(buffer.len());
            Some(String::from_utf16_lossy(&buffer[..end]))
        }
    }

    /// The event's message in words, from what its source installed for
    /// that. `None`: the source installed nothing (or was removed).
    fn message(event: &Handle, source: &str) -> Option<String> {
        let source = HSTRING::from(source);
        // SAFETY: as in `xml`: the size is asked first.
        unsafe {
            let metadata =
                Handle(EvtOpenPublisherMetadata(None, &source, PCWSTR::null(), 0, 0).ok()?);
            let mut used = 0u32;
            let sized = EvtFormatMessage(
                metadata.0,
                event.0,
                0,
                None,
                EvtFormatMessageEvent.0,
                None,
                &mut used,
            );
            match sized {
                Err(e) if e.code() == ERROR_INSUFFICIENT_BUFFER.to_hresult() && used > 0 => {}
                _ => return None,
            }
            let mut buffer = vec![0u16; used as usize];
            EvtFormatMessage(
                metadata.0,
                event.0,
                0,
                None,
                EvtFormatMessageEvent.0,
                Some(&mut buffer),
                &mut used,
            )
            .ok()?;
            let end = buffer.iter().position(|c| *c == 0).unwrap_or(buffer.len());
            let text = one_line(&String::from_utf16_lossy(&buffer[..end]));
            (!text.is_empty()).then_some(text)
        }
    }
}

#[cfg(not(windows))]
mod platform {
    use super::{Outcome, Query};

    pub async fn check(_query: Query<'_>) -> Outcome {
        Outcome::down("Event log checks work on Windows only")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::checks::Health;

    fn query<'a>(source: &'a str, ids: &'a [u32], level: EventLevel, minutes: u32) -> Query<'a> {
        Query {
            log: "System",
            source,
            ids,
            level,
            minutes,
        }
    }

    #[test]
    fn the_query_names_what_counts_and_how_far_back() {
        assert_eq!(
            xpath(&query("", &[], EventLevel::Error, 60)),
            "*[System[(Level=1 or Level=2) and TimeCreated[timediff(@SystemTime) <= 3600000]]]"
        );
        assert_eq!(
            xpath(&query("disk", &[7, 51, 153], EventLevel::Warning, 1440)),
            "*[System[Provider[@Name='disk'] and (EventID=7 or EventID=51 or EventID=153) \
             and (Level=1 or Level=2 or Level=3) \
             and TimeCreated[timediff(@SystemTime) <= 86400000]]]"
        );
        assert!(
            xpath(&query("", &[4], EventLevel::Information, 1)).contains("(Level>=0 and Level<=4)")
        );
        assert!(xpath(&query("", &[], EventLevel::Critical, 1)).contains("(Level=1) and"));
        // 31 days in milliseconds is more than 32 bits hold.
        assert!(xpath(&query("", &[], EventLevel::Error, 44_640)).contains("<= 2678400000]"));
    }

    #[test]
    fn an_event_is_read_from_the_xml_windows_renders() {
        let xml = "<Event xmlns='http://schemas.microsoft.com/win/2004/08/events/event'><System>\
            <Provider Name='disk'/><EventID Qualifiers='49156'>51</EventID><Level>3</Level>\
            <TimeCreated SystemTime='2026-10-12T14:02:11.123456700Z'/><Channel>System</Channel>\
            </System><EventData><Data>\\Device\\Harddisk1\\DR1</Data></EventData></Event>";
        assert_eq!(
            from_xml(xml),
            Some(Event {
                id: 51,
                source: "disk".into(),
                time: "2026-10-12 14:02".into(),
                text: String::new(),
            })
        );
        // A newer source: more on the provider, no qualifiers on the ID.
        let xml = "<Event><System><Provider Name=\"Microsoft-Windows-Backup\" \
            Guid=\"{1db28f2e-8f80-4027-8c5a-a11f7f10f62d}\" /><EventID>4</EventID>\
            <TimeCreated SystemTime=\"2026-10-11T02:30:00.0Z\" /></System></Event>";
        let event = from_xml(xml).unwrap();
        assert_eq!(
            (event.source.as_str(), event.id, event.time.as_str()),
            ("Microsoft-Windows-Backup", 4, "2026-10-11 02:30")
        );
        assert_eq!(
            from_xml(
                "<Event><System><Provider Name='R&amp;D Agent'/><EventID>1</EventID>\
                      <TimeCreated SystemTime='2026-01-01T00:00:00.0Z'/></System></Event>"
            )
            .unwrap()
            .source,
            "R&D Agent"
        );
        for broken in [
            "",
            "<Event/>",
            "<Event><System><EventID>x</EventID></System></Event>",
        ] {
            assert_eq!(from_xml(broken), None, "{broken}");
        }
    }

    #[test]
    fn what_was_found_is_said_with_the_latest_event() {
        let disk = Event {
            id: 51,
            source: "disk".into(),
            time: "2026-10-12 14:02".into(),
            text: "An error was detected on device \\Device\\Harddisk1\\DR1 during a paging operation."
                .into(),
        };
        let none = describe(0, None, 60);
        assert_eq!(none.status, Health::Up);
        assert_eq!(none.message, "No such event in the last 1 hour");
        assert_eq!(none.metrics[0].key, metric::EVENTS);
        assert_eq!(none.metrics[0].value, 0.0);

        let some = describe(3, Some(&disk), 1440);
        // Up: whether events are trouble is the hub's to say.
        assert_eq!(some.status, Health::Up);
        assert_eq!(
            some.message,
            "3 events in the last 1 day; the latest: disk 51 at 2026-10-12 14:02 UTC: \
             An error was detected on device \\Device\\Harddisk1\\DR1 during a paging operation."
        );
        assert_eq!(some.metrics[0].value, 3.0);

        // A source that left no words for its events.
        let wordless = Event {
            text: String::new(),
            ..disk.clone()
        };
        assert_eq!(
            describe(1, Some(&wordless), 45).message,
            "1 event in the last 45 minutes; the latest: disk 51 at 2026-10-12 14:02 UTC"
        );
        assert!(
            describe(MAX_EVENT_COUNT, Some(&wordless), 90)
                .message
                .starts_with("1000 events or more in the last 90 minutes;")
        );
        assert_eq!(
            describe(2, None, 120).message,
            "2 events in the last 2 hours"
        );

        assert_eq!(span(1), "1 minute");
        assert_eq!(span(10_080), "7 days");
        assert_eq!(one_line("  a\r\n\tb  c \0"), "a b c");
        assert_eq!(one_line(&"é".repeat(300)).chars().count(), 201);
    }

    #[cfg(not(windows))]
    #[tokio::test]
    async fn another_system_has_no_event_log() {
        let outcome = check(query("", &[], EventLevel::Error, 60)).await;
        assert_eq!(outcome.status, Health::Down);
        assert_eq!(outcome.message, "Event log checks work on Windows only");
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn this_machines_logs_are_read_and_a_missing_one_is_said() {
        // Every machine's System log has events of the last 31 days.
        let all = check(query("", &[], EventLevel::Information, 44_640)).await;
        assert_eq!(all.status, Health::Up, "{}", all.message);
        assert!(all.metrics[0].value >= 1.0, "{}", all.message);
        assert!(all.message.contains("; the latest: "), "{}", all.message);

        // An ID nothing writes: none, and that is an answer.
        let none = check(query("", &[64_999], EventLevel::Information, 60)).await;
        assert_eq!(none.status, Health::Up, "{}", none.message);
        assert_eq!(none.message, "No such event in the last 1 hour");

        let missing = check(Query {
            log: "Blentinel-No-Such-Log",
            ..query("", &[], EventLevel::Error, 60)
        })
        .await;
        assert_eq!(missing.status, Health::Down);
        assert_eq!(
            missing.message,
            "No event log named Blentinel-No-Such-Log on this machine"
        );
    }

    /// By hand (`--ignored --nocapture`): `EVENTLOG_TEST` is
    /// `log|source|ids|level|minutes` (`System|disk|7,51|warning|1440`).
    #[tokio::test]
    #[ignore = "reads this machine's event logs: run by hand"]
    async fn a_log_of_this_machine() {
        let asked = std::env::var("EVENTLOG_TEST").expect("EVENTLOG_TEST");
        let parts: Vec<&str> = asked.split('|').collect();
        let ids: Vec<u32> = parts[2]
            .split(',')
            .filter_map(|id| id.trim().parse().ok())
            .collect();
        let outcome = check(Query {
            log: parts[0],
            source: parts[1],
            ids: &ids,
            level: EventLevel::parse(parts[3]).expect("a level"),
            minutes: parts[4].parse().expect("minutes"),
        })
        .await;
        println!("{:?}: {}", outcome.status, outcome.message);
        println!(
            "events = {}",
            outcome.metrics.first().map_or(-1.0, |m| m.value)
        );
    }
}
