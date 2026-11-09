//! Fence mechanics for the shared documentation example loader.
//!
//! The parent module decides which fences are examples and in what order; this
//! module answers the narrower questions about a single line: does it open or
//! close a fence, what info string does it carry, and where does its body end.
//! Keeping those rules here leaves the parent free to walk the document and
//! keeps both files within the repository's module size limit.

use super::PublishedDocument;

/// A fence marker and the number of characters it repeats.
#[derive(Debug, Clone, Copy)]
pub(super) struct Fence {
    /// The fence character, either a backtick or a tilde.
    pub(super) marker: char,
    /// How many marker characters the fence repeats.
    pub(super) length: usize,
}

/// Return the fence when `line` opens one.
///
/// Markdown lets a fence be indented by up to three spaces while still opening
/// a block, and requires at least three marker characters, so both rules are
/// enforced here rather than at each call site.
pub(super) fn opening_fence(line: &str) -> Option<Fence> {
    let indentation = line
        .chars()
        .take_while(|character| *character == ' ')
        .count();
    if indentation > 3 {
        return None;
    }
    let remainder = line.get(indentation..)?;
    let marker = remainder.chars().next()?;
    if !matches!(marker, '`' | '~') {
        return None;
    }
    let length = remainder
        .chars()
        .take_while(|character| *character == marker)
        .count();
    (length >= 3).then_some(Fence { marker, length })
}

/// Return whether `line` closes `fence`.
///
/// A closing fence repeats at least as many marker characters as the opener and
/// carries no info string, so a shorter run inside the body is not mistaken for
/// the close.
pub(super) fn is_closing_fence(line: &str, fence: Fence) -> bool {
    let indentation = line
        .chars()
        .take_while(|character| *character == ' ')
        .count();
    if indentation > 3 {
        return false;
    }
    let Some(remainder) = line.get(indentation..) else {
        return false;
    };
    let length = remainder
        .chars()
        .take_while(|character| *character == fence.marker)
        .count();
    remainder
        .get(length..)
        .is_some_and(|suffix| length >= fence.length && suffix.trim().is_empty())
}

/// Return the info string that follows the fence marker on `line`.
pub(super) fn fence_info(line: &str, fence: Fence) -> &str {
    let indentation = line
        .chars()
        .take_while(|character| *character == ' ')
        .count();
    let Some(remainder) = line.get(indentation..) else {
        return "";
    };
    let info_start = remainder
        .chars()
        .take_while(|character| *character == fence.marker)
        .count();
    remainder.get(info_start..).unwrap_or_default().trim()
}

/// Return the text between the opening fence and `closing_line`.
///
/// The body keeps a trailing newline when it has any lines at all, so the value
/// matches the bytes a reader sees between the fences.
pub(super) fn fence_source(
    document: &PublishedDocument<'_>,
    start_line: usize,
    closing_line: usize,
) -> Option<String> {
    let mut source = document.lines().get(start_line..closing_line)?.join("\n");
    source.push_str(if start_line < closing_line { "\n" } else { "" });
    Some(source)
}

/// Return the identifier from a `<!-- tested-example: id -->` comment.
///
/// The surrounding spacing is part of the grammar: a marker with extra spaces or
/// missing delimiters is reported rather than silently accepted.
pub(super) fn parse_marker(line: &str) -> Option<String> {
    let comment = line.trim();
    let body = comment
        .strip_prefix("<!-- tested-example: ")?
        .strip_suffix(" -->")?;
    (!body.is_empty()).then(|| body.to_owned())
}
