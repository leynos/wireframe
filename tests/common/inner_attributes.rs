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
/// ```plaintext
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
    let mut collector = AttributeCollector::default();
    for line in source.lines().map(str::trim) {
        if !collector.feed(line) {
            break;
        }
    }
    collector.attributes
}

/// Whether a source file's header carries `attribute` as a standalone entry.
///
/// A prohibition nested inside another attribute is part of that attribute and
/// does not count, because a file in that state does not compile.
pub(crate) fn has_header_attribute(source: &str, attribute: &str) -> bool {
    header_inner_attributes(source)
        .iter()
        .any(|candidate| candidate == attribute)
}

/// Report whether a trimmed line may appear in a file header.
fn is_header_line(line: &str) -> bool {
    line.is_empty() || line.starts_with("//!") || line.starts_with("#![")
}

/// Return the parenthesis depth after reading `line`.
///
/// A line that closes more groups than it opens cannot push the depth below
/// zero, so a stray `)` in an attribute body cannot close it early.
fn adjust_depth(depth: usize, line: &str) -> usize {
    depth
        .saturating_add(line.matches('(').count())
        .saturating_sub(line.matches(')').count())
}

/// Joins header lines into one string per attribute.
#[derive(Default)]
struct AttributeCollector {
    attributes: Vec<String>,
    open: String,
    depth: usize,
}

impl AttributeCollector {
    /// Feed one trimmed source line, returning `false` once the header ends.
    ///
    /// Blank lines and `//!` documentation belong to the header but carry no
    /// attribute, and an attribute at depth zero starts collection.
    fn feed(&mut self, line: &str) -> bool {
        // A line inside an open attribute is a continuation whatever it looks
        // like; only at depth zero does a non-header line end the header.
        if self.depth == 0 {
            if !is_header_line(line) {
                return false;
            }
            if !line.starts_with("#![") {
                return true;
            }
        }
        self.add(line);
        true
    }

    /// Append `line` to the open attribute and close it at depth zero.
    fn add(&mut self, line: &str) {
        if !self.open.is_empty() {
            self.open.push(' ');
        }
        self.open.push_str(line);
        self.depth = adjust_depth(self.depth, line);
        if self.depth == 0 {
            self.attributes.push(std::mem::take(&mut self.open));
        }
    }
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

#[cfg(test)]
mod attribute_property_tests {
    //! Properties of the header parser over generated attribute sequences.
    //!
    //! The generator only produces inner attributes that could appear in a real
    //! file: a name followed by a parenthesized argument list, optionally wrapped
    //! over several lines. The malformed nesting that caused the original defect
    //! stays in the regression table above, because a parser that only has to
    //! handle valid syntax would not need to join continuation lines at all.

    use proptest::prelude::*;

    use super::{has_header_attribute, header_inner_attributes};

    /// The prohibition the contract test looks for.
    const PROHIBITION: &str = "#![forbid(unsafe_code)]";

    /// An argument list of `count` generated entries, one per line.
    fn attribute_body(count: usize) -> String {
        let mut body = String::new();
        for index in 0..count {
            body.push_str("\n    argument");
            body.push_str(&index.to_string());
            body.push(',');
        }
        body
    }

    /// A generated inner attribute with between zero and three arguments.
    fn attribute() -> impl Strategy<Value = String> {
        (0usize..3, "[a-z_]{1,8}")
            .prop_map(|(arguments, name)| format!("#![{name}({})]", attribute_body(arguments)))
    }

    /// A generated attribute whose argument list quotes the prohibition.
    ///
    /// The text is nested inside the attribute but is data, not an attribute of
    /// its own, so the parser must not count it as a standalone entry.
    fn attribute_quoting_the_prohibition() -> impl Strategy<Value = String> {
        (0usize..3, "[a-z_]{1,8}").prop_map(|(arguments, name)| {
            format!(
                "#![{name}({}    reason = \"{PROHIBITION}\"\n)]",
                attribute_body(arguments)
            )
        })
    }

    proptest! {
        #[test]
        fn a_standalone_prohibition_after_the_attributes_counts(
            attributes in prop::collection::vec(attribute(), 0..4),
        ) {
            let mut source = String::new();
            for generated in &attributes {
                source.push_str(generated);
                source.push('\n');
            }
            source.push_str(PROHIBITION);
            source.push('\n');

            let parsed = header_inner_attributes(&source);
            prop_assert_eq!(parsed.len(), attributes.len() + 1);
            prop_assert_eq!(parsed.last().map(String::as_str), Some(PROHIBITION));
            prop_assert!(has_header_attribute(&source, PROHIBITION));
        }

        #[test]
        fn stops_at_the_first_item(
            attributes in prop::collection::vec(attribute(), 0..4),
            item in "[a-z_]{1,8}",
        ) {
            let mut source = String::new();
            for generated in &attributes {
                source.push_str(generated);
                source.push('\n');
            }
            source.push_str(&item);
            source.push('\n');
            source.push_str(PROHIBITION);
            source.push('\n');

            let parsed = header_inner_attributes(&source);
            prop_assert_eq!(parsed.len(), attributes.len());
            prop_assert!(parsed.iter().all(|entry| entry.starts_with("#![")));
            prop_assert!(!has_header_attribute(&source, PROHIBITION));
        }

        #[test]
        fn a_prohibition_quoted_inside_an_attribute_never_counts(
            attribute in attribute_quoting_the_prohibition(),
        ) {
            let source = format!("{attribute}\n");

            let parsed = header_inner_attributes(&source);
            prop_assert_eq!(parsed.len(), 1);
            prop_assert!(!has_header_attribute(&source, PROHIBITION));
        }
    }
}
