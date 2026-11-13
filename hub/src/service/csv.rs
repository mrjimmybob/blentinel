//! CSV for the hub's downloads (the audit log, the monthly report).

/// A text field: quoted when it needs to be, and never read as a formula by
/// a spreadsheet (a value that starts with `=`, `+`, `-` or `@` gets a `'`
/// in front). Names and messages come from people and probes.
pub fn field(text: &str) -> String {
    let text = if text.starts_with(['=', '+', '-', '@']) {
        format!("'{text}")
    } else {
        text.to_string()
    };
    if text.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", text.replace('"', "\"\""))
    } else {
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fields_are_quoted_when_needed_and_never_formulas() {
        assert_eq!(field("plain"), "plain");
        assert_eq!(field("ACME, Ltd"), "\"ACME, Ltd\"");
        assert_eq!(field(r#"say "hi""#), r#""say ""hi""""#);
        assert_eq!(field("=HYPERLINK(\"x\")"), "\"'=HYPERLINK(\"\"x\"\")\"");
        assert_eq!(field("+1"), "'+1");
        assert_eq!(field("@sum"), "'@sum");
        assert_eq!(field(""), "");
    }
}
