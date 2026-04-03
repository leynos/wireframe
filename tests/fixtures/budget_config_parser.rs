//! Shared parsing helpers for budget fixture configuration strings.

use std::{fmt, str::FromStr};

/// Parsed fields from
/// "`timeout_ms` / `per_message` / `per_connection` / `in_flight`".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StandardBudgetConfig {
    /// Send timeout in milliseconds for each operation.
    ///
    /// This must be a non-negative integer that fits in `u64`.
    pub timeout_ms: u64,
    /// Maximum payload size in bytes allowed for a single message.
    ///
    /// Fixture scenarios typically use positive values to avoid immediate
    /// budget rejection.
    pub per_message: usize,
    /// Maximum aggregate bytes that may be buffered per connection.
    ///
    /// This represents the connection-level memory ceiling used by budget
    /// checks.
    pub per_connection: usize,
    /// Maximum number of in-flight messages permitted per connection.
    ///
    /// Values greater than zero allow concurrency; lower values tighten back
    /// pressure behaviour in fixture scenarios.
    pub in_flight: usize,
}

/// Parse the standard four-segment budget config string used by budget BDD
/// fixtures.
pub fn parse_standard_budget_config(s: &str) -> Result<StandardBudgetConfig, String> {
    let mut values = s.split('/').map(str::trim);
    let timeout_ms = next_non_empty(&mut values, "timeout_ms")?;
    let per_message = next_non_empty(&mut values, "per_message")?;
    let per_connection = next_non_empty(&mut values, "per_connection")?;
    let in_flight = next_non_empty(&mut values, "in_flight")?;
    if values.next().is_some() {
        return Err("unexpected trailing segments".to_string());
    }

    Ok(StandardBudgetConfig {
        timeout_ms: parse_segment(timeout_ms, "timeout_ms")?,
        per_message: parse_segment(per_message, "per_message")?,
        per_connection: parse_segment(per_connection, "per_connection")?,
        in_flight: parse_segment(in_flight, "in_flight")?,
    })
}

fn next_non_empty<'a>(
    values: &mut impl Iterator<Item = &'a str>,
    name: &str,
) -> Result<&'a str, String> {
    values
        .next()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("missing {name}"))
}

fn parse_segment<T: FromStr>(value: &str, name: &str) -> Result<T, String>
where
    T::Err: fmt::Display,
{
    value.parse().map_err(|error| format!("{name}: {error}"))
}

#[cfg(test)]
mod tests {
    //! Coverage for standard budget fixture parsing edge cases.

    use super::{StandardBudgetConfig, parse_standard_budget_config};

    #[test]
    fn parse_standard_budget_config_parses_valid_input() {
        let Ok(parsed) = parse_standard_budget_config("200/2048/8/8") else {
            panic!("valid input should parse");
        };
        assert_eq!(
            parsed,
            StandardBudgetConfig {
                timeout_ms: 200,
                per_message: 2048,
                per_connection: 8,
                in_flight: 8,
            }
        );
    }

    #[test]
    fn parse_standard_budget_config_reports_missing_in_flight_segment() {
        assert_eq!(
            parse_standard_budget_config("10/64/100").expect_err("in_flight should be required"),
            "missing in_flight"
        );
    }

    #[test]
    fn parse_standard_budget_config_reports_trailing_segments() {
        assert_eq!(
            parse_standard_budget_config("10/64/100/100/extra")
                .expect_err("unexpected segments should fail"),
            "unexpected trailing segments"
        );
    }

    #[test]
    fn parse_standard_budget_config_reports_timeout_parse_failure_prefix() {
        let error = parse_standard_budget_config("abc/64/100/100")
            .expect_err("timeout parse failure should be reported");
        assert!(
            error.starts_with("timeout_ms: "),
            "expected timeout prefix, got: {error}"
        );
    }
}
