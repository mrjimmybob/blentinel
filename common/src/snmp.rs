//! The values of an `snmp` check as people write them, one a line (M5-09,
//! D-87). The check form shows and reads this text, and the hub's SNMP
//! templates are written in it:
//!
//! ```text
//! 1.3.6.1.2.1.1.3.0
//! Uplink = 1.3.6.1.2.1.2.2.1.8.1 = 1
//! Toner = 1.3.6.1.2.1.43.11.1.1.9 ; rows 1.3.6.1.2.1.43.11.1.1.6 ; of 1.3.6.1.2.1.43.11.1.1.8 ; warn < 20 ; down < 5
//! ```
//!
//! A line is `Name = OID = what it must be` (the name and the expectation
//! are optional; several accepted values are separated by `|`), then
//! options after `;`:
//!
//! | Option | Meaning |
//! |---|---|
//! | `walk` | the OID is a column of a table: every row |
//! | `rows OID` | a table whose rows are named by that column |
//! | `of OID` | a percentage of that value (its capacity) |
//! | `per second` | a counter: how fast it grows |
//! | `x 0.1`, `/ 60` | multiply or divide the number |
//! | `unit °C` | its unit |
//! | `means 1=up, 2=down` | what its values mean |
//! | `not down \| failed` | what it must not be |
//! | `warn > 60`, `down < 5` | the limits the hub judges it by |

use std::collections::BTreeMap;

use crate::checks::{SnmpLimit, SnmpOid, oid_arcs};

/// Every line of `text` as a value; empty lines are skipped. `Err` says
/// which line can't be read, and why.
pub fn parse_values(text: &str) -> Result<Vec<SnmpOid>, String> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(parse_value)
        .collect()
}

/// The values as lines [`parse_values`] reads back the same.
pub fn values_text(values: &[SnmpOid]) -> String {
    values.iter().map(value_text).collect::<Vec<_>>().join("\n")
}

fn words(text: &str, separator: char) -> Vec<String> {
    text.split(separator)
        .map(|word| word.trim().to_string())
        .collect()
}

/// One line as a value. What the line says is not judged here
/// ([`SnmpOid::validate`] does), only whether it can be read.
pub fn parse_value(line: &str) -> Result<SnmpOid, String> {
    let mut parts = line.split(';').map(str::trim);
    let head = parts.next().unwrap_or_default();
    let fields: Vec<&str> = head.splitn(3, '=').map(str::trim).collect();
    let (name, oid, expect) = match fields.as_slice() {
        [name, oid, expect] => (*name, *oid, Some(*expect)),
        // Two parts: an OID first means it is followed by what it must be.
        [first, second] if oid_arcs(first).is_some() => ("", *first, Some(*second)),
        [name, oid] => (*name, *oid, None),
        _ => ("", head, None),
    };
    let mut value = SnmpOid {
        oid: oid.to_string(),
        name: name.to_string(),
        expect: expect.map(|text| words(text, '|')).unwrap_or_default(),
        ..SnmpOid::default()
    };
    for option in parts.filter(|option| !option.is_empty()) {
        let (keyword, rest) = match option.split_once(char::is_whitespace) {
            Some((keyword, rest)) => (keyword, rest.trim()),
            None => (option, ""),
        };
        let wrong = |why: &str| format!("{}: \"{option}\": {why}", value_label(name, oid));
        let needs = |what: &str| -> Result<String, String> {
            if rest.is_empty() {
                Err(wrong(&format!("{what} is missing")))
            } else {
                Ok(rest.to_string())
            }
        };
        let number = |text: &str| -> Result<f64, String> {
            text.trim()
                .parse::<f64>()
                .ok()
                .filter(|n| n.is_finite())
                .ok_or_else(|| wrong("a number is expected"))
        };
        match keyword.to_ascii_lowercase().as_str() {
            "walk" if rest.is_empty() => value.walk = true,
            "rows" => {
                value.walk = true;
                value.rows = Some(needs("the OID of the column that names the rows")?);
            }
            "of" => value.of = Some(needs("the OID of what it is a share of")?),
            "per" if rest.eq_ignore_ascii_case("second") => value.rate = true,
            "x" | "*" => value.scale = Some(number(rest)?),
            "/" => {
                let by = number(rest)?;
                if by == 0.0 {
                    return Err(wrong("nothing is divided by 0"));
                }
                value.scale = Some(1.0 / by);
            }
            "unit" => value.unit = needs("the unit")?,
            "means" => {
                for pair in rest.split(',').map(str::trim).filter(|p| !p.is_empty()) {
                    let (is, meaning) = pair
                        .split_once('=')
                        .ok_or_else(|| wrong("meanings are written 1=up, 2=down"))?;
                    value
                        .names
                        .insert(is.trim().to_string(), meaning.trim().to_string());
                }
                if value.names.is_empty() {
                    return Err(wrong("meanings are written 1=up, 2=down"));
                }
            }
            "not" => value.not = words(&needs("what it must not be")?, '|'),
            "warn" | "down" => {
                let limit = match rest.split_at_checked(1) {
                    Some(("<", at)) => SnmpLimit::Under(number(at)?),
                    Some((">", at)) => SnmpLimit::Over(number(at)?),
                    _ => return Err(wrong("a limit is written < 20 or > 60")),
                };
                if keyword.eq_ignore_ascii_case("warn") {
                    value.warn = Some(limit);
                } else {
                    value.down = Some(limit);
                }
            }
            _ => {
                return Err(wrong(
                    "not an option (walk, rows, of, per second, x, /, unit, means, not, warn, down)",
                ));
            }
        }
    }
    Ok(value)
}

fn value_label<'a>(name: &'a str, oid: &'a str) -> &'a str {
    if name.is_empty() { oid } else { name }
}

fn limit_text(limit: SnmpLimit) -> String {
    match limit {
        SnmpLimit::Over(at) => format!("> {at}"),
        SnmpLimit::Under(at) => format!("< {at}"),
    }
}

/// A factor as it is written: `/ 60` for a small whole divisor (not
/// `x 0.016666666666666666`), else `x 0.000008`.
fn scale_text(scale: f64) -> String {
    let by = (1.0 / scale).round();
    if (2.0..=1000.0).contains(&by) && 1.0 / by == scale {
        format!("/ {by}")
    } else {
        format!("x {scale}")
    }
}

/// One value as a line.
pub fn value_text(value: &SnmpOid) -> String {
    let mut line = String::new();
    if !value.name.is_empty() {
        line.push_str(&format!("{} = ", value.name));
    }
    line.push_str(&value.oid);
    if !value.expect.is_empty() {
        line.push_str(&format!(" = {}", value.expect.join(" | ")));
    }
    let mut options = Vec::new();
    match &value.rows {
        Some(rows) => options.push(format!("rows {rows}")),
        None if value.walk => options.push("walk".to_string()),
        None => {}
    }
    if let Some(of) = &value.of {
        options.push(format!("of {of}"));
    }
    if value.rate {
        options.push("per second".to_string());
    }
    if let Some(scale) = value.scale {
        options.push(scale_text(scale));
    }
    if !value.unit.is_empty() {
        options.push(format!("unit {}", value.unit));
    }
    if !value.names.is_empty() {
        options.push(format!("means {}", meanings_text(&value.names)));
    }
    if !value.not.is_empty() {
        options.push(format!("not {}", value.not.join(" | ")));
    }
    if let Some(warn) = value.warn {
        options.push(format!("warn {}", limit_text(warn)));
    }
    if let Some(down) = value.down {
        options.push(format!("down {}", limit_text(down)));
    }
    for option in options {
        line.push_str(&format!(" ; {option}"));
    }
    line
}

/// `1=up, 2=down`, numbers in their order (not `1, 10, 11, 2`).
fn meanings_text(names: &BTreeMap<String, String>) -> String {
    let mut pairs: Vec<(&String, &String)> = names.iter().collect();
    pairs.sort_by(
        |(a, _), (b, _)| match (a.parse::<i64>(), b.parse::<i64>()) {
            (Ok(a), Ok(b)) => a.cmp(&b),
            _ => a.cmp(b),
        },
    );
    pairs
        .iter()
        .map(|(is, meaning)| format!("{is}={meaning}"))
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_is_an_oid_with_a_name_and_what_it_must_be() {
        let values = parse_values(
            "1.3.6.1.2.1.1.3.0\n\n  Uplink = 1.3.6.1.2.1.2.2.1.8.1 = 1 \n1.3.6.1.2.1.1.5.0 = core\nName=1.3.6.1.2.1.1.5.0\nState = 1.3.6.1.2.1.25.3.5.1.1.1 = 3 | 4|5",
        )
        .unwrap();
        assert_eq!(values.len(), 5);
        assert_eq!(values[0], SnmpOid::of("1.3.6.1.2.1.1.3.0"));
        assert_eq!(
            (values[1].name.as_str(), values[1].expect.as_slice()),
            ("Uplink", &["1".to_string()][..])
        );
        assert_eq!(
            (values[2].name.as_str(), values[2].expect.as_slice()),
            ("", &["core".to_string()][..])
        );
        assert_eq!(
            (values[3].name.as_str(), values[3].expect.len()),
            ("Name", 0)
        );
        assert_eq!(values[4].expect, ["3", "4", "5"]);
        // What isn't an OID is kept as typed, for validation to refuse.
        assert_eq!(parse_values("sysUpTime.0").unwrap()[0].oid, "sysUpTime.0");
        assert!(parse_values(" \n ").unwrap().is_empty());
    }

    #[test]
    fn options_say_how_a_value_is_read() {
        let toner = parse_value(
            "Toner = 1.3.6.1.2.1.43.11.1.1.9 ; rows 1.3.6.1.2.1.43.11.1.1.6 ; of 1.3.6.1.2.1.43.11.1.1.8 ; warn < 20 ; down < 5",
        )
        .unwrap();
        assert!(toner.walk);
        assert_eq!(toner.rows.as_deref(), Some("1.3.6.1.2.1.43.11.1.1.6"));
        assert_eq!(toner.of.as_deref(), Some("1.3.6.1.2.1.43.11.1.1.8"));
        assert_eq!(toner.warn, Some(SnmpLimit::Under(20.0)));
        assert_eq!(toner.down, Some(SnmpLimit::Under(5.0)));
        assert_eq!(toner.validate(), Ok(()));

        let traffic = parse_value(
            "In = 1.3.6.1.2.1.31.1.1.1.6.1 ; per second ; x 0.000008 ; unit Mbit/s ; WARN >800",
        )
        .unwrap();
        assert!(traffic.rate && !traffic.walk);
        assert_eq!(traffic.scale, Some(0.000008));
        assert_eq!(traffic.unit, "Mbit/s");
        assert_eq!(traffic.warn, Some(SnmpLimit::Over(800.0)));

        let runtime = parse_value("1.3.6.1.4.1.318.1.1.1.2.2.3.0 ; / 60 ; unit min").unwrap();
        assert_eq!(runtime.scale, Some(1.0 / 60.0));

        let state = parse_value(
            "Volume = 1.3.6.1.4.1.6574.3.1.1.3 ; walk ; means 1=normal, 11=degraded, 12=crashed ; not degraded | crashed",
        )
        .unwrap();
        assert!(state.walk && state.rows.is_none());
        assert_eq!(state.names["11"], "degraded");
        assert_eq!(state.not, ["degraded", "crashed"]);
        assert_eq!(state.validate(), Ok(()));
    }

    #[test]
    fn a_line_that_cant_be_read_says_why() {
        let error = |line: &str| parse_value(line).unwrap_err();
        assert_eq!(
            error("Toner = 1.3.6.1 ; walks"),
            "Toner: \"walks\": not an option (walk, rows, of, per second, x, /, unit, means, not, warn, down)"
        );
        assert_eq!(
            error("1.3.6.1 ; rows"),
            "1.3.6.1: \"rows\": the OID of the column that names the rows is missing"
        );
        assert_eq!(
            error("1.3.6.1 ; warn 20"),
            "1.3.6.1: \"warn 20\": a limit is written < 20 or > 60"
        );
        assert!(error("1.3.6.1 ; down < low").ends_with("a number is expected"));
        assert!(error("1.3.6.1 ; x ten").ends_with("a number is expected"));
        assert!(error("1.3.6.1 ; / 0").ends_with("nothing is divided by 0"));
        assert!(error("1.3.6.1 ; means up").ends_with("meanings are written 1=up, 2=down"));
        assert!(error("1.3.6.1 ; per minute").contains("not an option"));
        // The first bad line of several is the one named.
        assert!(
            parse_values("1.3.6.1.2.1.1.3.0\nFan = 1.3.6.1 ; unit")
                .unwrap_err()
                .starts_with("Fan: ")
        );
    }

    #[test]
    fn values_are_written_as_lines_that_read_back_the_same() {
        let text = "1.3.6.1.2.1.1.3.0\n\
            Uplink = 1.3.6.1.2.1.2.2.1.8.1 = 1\n\
            1.3.6.1.2.1.1.5.0 = core | edge\n\
            Toner = 1.3.6.1.2.1.43.11.1.1.9 ; rows 1.3.6.1.2.1.43.11.1.1.6 ; of 1.3.6.1.2.1.43.11.1.1.8 ; warn < 20 ; down < 5\n\
            In = 1.3.6.1.2.1.31.1.1.1.6 ; walk ; per second ; x 0.000008 ; unit Mbit/s\n\
            Runtime = 1.3.6.1.4.1.318.1.1.1.2.2.3.0 ; / 60 ; unit min ; warn < 15.5\n\
            Volume = 1.3.6.1.4.1.6574.3.1.1.3 ; walk ; means 1=normal, 2=repairing, 11=degraded ; not degraded";
        let values = parse_values(text).unwrap();
        assert_eq!(values_text(&values), text);
        assert_eq!(parse_values(&values_text(&values)).unwrap(), values);
        for value in &values {
            assert_eq!(value.validate(), Ok(()), "{}", value.label());
        }
    }
}
