//! Shared parsing helpers for budget fixture configuration strings.

use std::{fmt, str::FromStr};

/// Parsed fields from
/// "`timeout_ms` / `per_message` / `per_connection` / `in_flight`".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StandardBudgetConfig {
    pub timeout_ms: u64,
    pub per_message: usize,
    pub per_connection: usize,
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
