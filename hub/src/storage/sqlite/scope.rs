//! A user's company scope as SQL (D-45): lists are filtered here, in the
//! storage layer, so no list endpoint can return another company's data.

use crate::model::CompanyScope;

/// A `WHERE` condition on `column` for `scope`, with its `?` bind values in order.
pub(super) fn condition(column: &str, scope: &CompanyScope) -> (String, Vec<String>) {
    let placeholders = |n: usize| vec!["?"; n].join(", ");
    match scope {
        CompanyScope::All => ("1 = 1".to_string(), Vec::new()),
        CompanyScope::Only(ids) if ids.is_empty() => ("1 = 0".to_string(), Vec::new()),
        CompanyScope::Only(ids) => (
            format!("{column} IN ({})", placeholders(ids.len())),
            ids.iter().cloned().collect(),
        ),
        CompanyScope::AllExcept(ids) if ids.is_empty() => ("1 = 1".to_string(), Vec::new()),
        CompanyScope::AllExcept(ids) => (
            format!("{column} NOT IN ({})", placeholders(ids.len())),
            ids.iter().cloned().collect(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_mode_becomes_a_condition() {
        let ids = |v: &[&str]| v.iter().map(|s| s.to_string()).collect();
        assert_eq!(condition("c.id", &CompanyScope::All).0, "1 = 1");
        assert_eq!(
            condition("c.id", &CompanyScope::Only(ids(&["a", "b"]))),
            ("c.id IN (?, ?)".to_string(), vec!["a".into(), "b".into()])
        );
        assert_eq!(condition("c.id", &CompanyScope::Only(ids(&[]))).0, "1 = 0");
        assert_eq!(
            condition("c.id", &CompanyScope::AllExcept(ids(&["z"]))).0,
            "c.id NOT IN (?)"
        );
    }
}
