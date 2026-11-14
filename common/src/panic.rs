//! A panic, written to the log (D-117).
//!
//! The hub and the probe are built to unwind: a panic ends the task it
//! happened in, not the program. Rust's own message for it goes to stderr,
//! which a Windows service doesn't have, so it is also logged here.

use std::panic::PanicHookInfo;
use std::sync::Once;

/// Log every panic as an error (thread, place, message), then let the
/// previous hook do what it did. Call once logging is set up; later calls
/// change nothing.
pub fn log_panics() {
    static INSTALLED: Once = Once::new();
    INSTALLED.call_once(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            tracing::error!("{}", describe(info));
            previous(info);
        }));
    });
}

/// "A panic in thread `name` at file:line: message".
fn describe(info: &PanicHookInfo<'_>) -> String {
    let thread = std::thread::current();
    let thread = thread.name().unwrap_or("unnamed");
    let place = info
        .location()
        .map(|l| format!(" at {}:{}", l.file(), l.line()))
        .unwrap_or_default();
    format!(
        "A panic in thread `{thread}`{place}: {}",
        message(info.payload())
    )
}

/// What the panic said: `panic!` gives a `&str` or a `String`.
fn message(payload: &(dyn std::any::Any + Send)) -> &str {
    payload
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
        .unwrap_or("(no message)")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_panics_message_is_read_whatever_its_kind() {
        let literal = std::panic::catch_unwind(|| panic!("plain")).unwrap_err();
        assert_eq!(message(literal.as_ref()), "plain");
        let formatted = std::panic::catch_unwind(|| panic!("formatted {}", 7)).unwrap_err();
        assert_eq!(message(formatted.as_ref()), "formatted 7");
        let other = std::panic::catch_unwind(|| std::panic::panic_any(7_u8)).unwrap_err();
        assert_eq!(message(other.as_ref()), "(no message)");
    }
}
