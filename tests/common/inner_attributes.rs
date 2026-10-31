//! Read the inner attributes that form a Rust source file's header.
//!
//! The unsafe-code contract inspects target roots as text rather than compiling
//! them, so it needs its own small parser — and a naive one fails in a way that
//! hides real breakage. Scanning line by line treats every `#![...]` line as a
//! separate attribute, so a prohibition inserted *inside* a multi-line
//! `#![expect(...)]` reads as a prohibition even though the file no longer
//! compiles. That mistake was made once, in `tests/client_lifecycle.rs`, and it
//! survived every check that only looked for the attribute's text.
//!
//! Joining an attribute's continuation lines into a single entry keeps the
//! difference visible: the nested prohibition becomes part of the surrounding
//! attribute rather than an entry of its own, so the contract fails.

/// Return a source file's header inner attributes, one joined string each.
///
/// Continuation lines are joined with single spaces, so an attribute nested
/// inside another never appears as an entry of its own. The header ends at the
/// first line that is neither blank, `//!` documentation, nor part of an
/// attribute that is still open — an inner attribute below an item would not
/// compile, so stopping there keeps a buried prohibition from counting.
///
/// # Examples
///
/// ```ignore
/// let nested = "#![forbid(unsafe_code)]\n#![expect(\n    dead_code\n)]\n";
/// assert_eq!(
///     header_inner_attributes(nested),
///     ["#![forbid(unsafe_code)]", "#![expect( dead_code )]"]
/// );
///
/// // The same two lines with the prohibition inside the open attribute are one
/// // entry, and no entry equals the prohibition on its own.
/// let swallowed = "#![expect(\n#![forbid(unsafe_code)]\n    dead_code\n)]\n";
/// assert_eq!(
///     header_inner_attributes(swallowed),
///     ["#![expect( #![forbid(unsafe_code)] dead_code )]"]
/// );
/// ```
pub(crate) fn header_inner_attributes(source: &str) -> Vec<String> {
    let mut attributes = Vec::new();
    let mut open = String::new();
    let mut depth = 0usize;
    for line in source.lines().map(str::trim) {
        let is_header_line = line.is_empty() || line.starts_with("//!") || line.starts_with("#![");
        if depth == 0 && !is_header_line {
            break;
        }
        if depth == 0 && !line.starts_with("#![") {
            continue;
        }
        if !open.is_empty() {
            open.push(' ');
        }
        open.push_str(line);
        depth += line.matches('(').count();
        depth = depth.saturating_sub(line.matches(')').count());
        if depth == 0 {
            attributes.push(std::mem::take(&mut open));
        }
    }
    attributes
}

#[cfg(test)]
mod attribute_tests {
    //! Regressions for the continuation rule.
    //!
    //! The second case is the defect this module exists to prevent: the
    //! prohibition is present in the source, but only as text inside another
    //! attribute, where it does not forbid anything.

    use rstest::rstest;

    use super::header_inner_attributes;

    #[rstest]
    #[case::separate_attributes(
        "#![forbid(unsafe_code)]\n#![expect(\n    dead_code\n)]\n",
        &["#![forbid(unsafe_code)]", "#![expect( dead_code )]"]
    )]
    #[case::nested_inside_another_attribute(
        "#![expect(\n#![forbid(unsafe_code)]\n    dead_code\n)]\n",
        &["#![expect( #![forbid(unsafe_code)] dead_code )]"]
    )]
    fn joins_continuation_lines_into_one_attribute(
        #[case] source: &str,
        #[case] expected: &[&str],
    ) {
        assert_eq!(
            header_inner_attributes(source),
            expected
                .iter()
                .copied()
                .map(str::to_owned)
                .collect::<Vec<_>>()
        );
    }
}
