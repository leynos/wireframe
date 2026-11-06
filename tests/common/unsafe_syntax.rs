//! Detect prohibited `unsafe` syntax in a source text.
//!
//! The unsafe-code contract reads source files as text rather than compiling
//! every feature combination, so the detector has to state what it counts. It
//! strips comments first, because Rust treats a comment as whitespace and
//! `unsafe/* SAFETY */ fn` is therefore an unsafe function. It then finds the
//! `unsafe` keyword as a whole word and classifies what follows, so a construct
//! split across a line break, an attribute whose parenthesis is separated by a
//! space, and a construct separated by a comment are all reported. String and
//! character literals are copied verbatim, so a literal that spells a construct
//! out is reported: the check fails closed rather than quietly ignoring a real
//! one. Files that exist to name the constructs — this module and the contract
//! test — are exempted by the caller, which is why the keyword is spelled out
//! here rather than hidden behind a pattern.

/// The keyword every prohibited construct is built around.
const UNSAFE_KEYWORD: [char; 6] = ['u', 'n', 's', 'a', 'f', 'e'];

/// Unsafe-code violations in one source text.
///
/// An `unsafe_code` allowance is always a violation: `forbid` admits no
/// override, so the allowance cannot take effect and its presence means a root
/// is trying to weaken the prohibition. A scoped expectation and an unsafe
/// construct are violations too, because the workspace contains no sanctioned
/// unsafe code.
pub(crate) fn violations_in(source: &str, label: &str) -> Vec<String> {
    let code = strip_comments(source);
    let normalized_source = collapse_whitespace(&code);
    let mut violations = Vec::new();
    if lint_attribute_names_unsafe_code(&normalized_source, "allow(") {
        violations.push(format!(
            "{label} uses an `unsafe_code` allowance; the workspace forbids unsafe code"
        ));
    }
    if lint_attribute_names_unsafe_code(&normalized_source, "expect(") {
        violations.push(format!(
            "{label} carries an `unsafe_code` expectation; the workspace forbids unsafe code"
        ));
    }
    violations.extend(unsafe_constructs(&code).into_iter().map(|construct| {
        format!("{label} contains {construct} but the workspace forbids unsafe code")
    }));
    violations
}

/// Replace every Rust comment with a single space.
///
/// Rust treats a comment as whitespace, so `unsafe/* SAFETY */fn` is an unsafe
/// function and must be reported. String, character, and raw-string literals
/// are copied verbatim: a literal such as `"src/**/*.rs"` must not start a
/// block comment that would swallow the code after it.
fn strip_comments(source: &str) -> String {
    let characters: Vec<char> = source.chars().collect();
    let mut stripped = String::with_capacity(source.len());
    let mut index = 0;
    while let Some(character) = characters.get(index) {
        index = strip_next(&characters, index, *character, &mut stripped);
    }
    stripped
}

/// Copy or skip the token that starts at `start`, returning the next index.
fn strip_next(characters: &[char], start: usize, character: char, output: &mut String) -> usize {
    match character {
        '/' if is_line_comment_start(characters, start) => {
            output.push(' ');
            skip_line_comment(characters, start)
        }
        '/' if is_block_comment_start(characters, start) => {
            output.push(' ');
            skip_block_comment(characters, start)
        }
        '"' => copy_quoted(characters, start, '"', output),
        '\'' => copy_character_literal(characters, start, output),
        'r' if raw_string_hashes(characters, start).is_some() => {
            copy_raw_string(characters, start, output)
        }
        _ => {
            output.push(character);
            start + 1
        }
    }
}

/// Whether a line comment starts at `start`.
fn is_line_comment_start(characters: &[char], start: usize) -> bool {
    characters.get(start) == Some(&'/') && characters.get(start + 1) == Some(&'/')
}

/// Whether a block comment starts at `start`.
fn is_block_comment_start(characters: &[char], start: usize) -> bool {
    characters.get(start) == Some(&'/') && characters.get(start + 1) == Some(&'*')
}

/// Return the index of the line break that ends the comment at `start`.
fn skip_line_comment(characters: &[char], start: usize) -> usize {
    characters
        .iter()
        .enumerate()
        .skip(start)
        .find(|(_, character)| **character == '\n')
        .map_or(characters.len(), |(index, _)| index)
}

/// Return the index after the block comment that starts at `start`.
///
/// Rust nests block comments, so the scan tracks depth rather than stopping at
/// the first `*/`.
fn skip_block_comment(characters: &[char], start: usize) -> usize {
    let mut index = start + 2;
    let mut depth = 1;
    while let Some(character) = characters.get(index) {
        if *character == '/' && characters.get(index + 1) == Some(&'*') {
            depth += 1;
            index += 2;
        } else if *character == '*' && characters.get(index + 1) == Some(&'/') {
            depth -= 1;
            index += 2;
            if depth == 0 {
                return index;
            }
        } else {
            index += 1;
        }
    }
    characters.len()
}

/// Copy the literal delimited by `quote` that starts at `start`.
fn copy_quoted(characters: &[char], start: usize, quote: char, output: &mut String) -> usize {
    output.push(quote);
    let mut index = start + 1;
    while let Some(character) = characters.get(index) {
        output.push(*character);
        index += 1;
        if *character == '\\' {
            if let Some(escaped) = characters.get(index) {
                output.push(*escaped);
                index += 1;
            }
        } else if *character == quote {
            break;
        }
    }
    index
}

/// Copy a character literal, or just the apostrophe of a lifetime.
fn copy_character_literal(characters: &[char], start: usize, output: &mut String) -> usize {
    if is_character_literal(characters, start) {
        copy_quoted(characters, start, '\'', output)
    } else {
        output.push('\'');
        start + 1
    }
}

/// Whether the apostrophe at `start` opens a character literal.
fn is_character_literal(characters: &[char], start: usize) -> bool {
    match characters.get(start + 1) {
        Some('\\') => true,
        Some(_) => characters.get(start + 2) == Some(&'\''),
        None => false,
    }
}

/// The number of `#` characters in a raw-string opener at `start`, if any.
fn raw_string_hashes(characters: &[char], start: usize) -> Option<usize> {
    if characters.get(start) != Some(&'r') {
        return None;
    }
    let hashes = characters
        .iter()
        .skip(start + 1)
        .take_while(|character| **character == '#')
        .count();
    (characters.get(start + 1 + hashes) == Some(&'"')).then_some(hashes)
}

/// Copy the raw string that starts at `start`.
fn copy_raw_string(characters: &[char], start: usize, output: &mut String) -> usize {
    let hashes = raw_string_hashes(characters, start).unwrap_or_default();
    let opening_quote = start + 1 + hashes;
    let mut index = opening_quote + 1;
    for character in characters.iter().skip(start).take(index - start) {
        output.push(*character);
    }
    while let Some(character) = characters.get(index) {
        output.push(*character);
        index += 1;
        if *character == '"' && closes_raw_string(characters, index, hashes) {
            return copy_closing_hashes(characters, index, hashes, output);
        }
    }
    index
}

/// Copy the `hashes` closing `#` characters at `index`, returning the new index.
fn copy_closing_hashes(
    characters: &[char],
    mut index: usize,
    hashes: usize,
    output: &mut String,
) -> usize {
    for _ in 0..hashes {
        let Some(hash) = characters.get(index) else {
            break;
        };
        output.push(*hash);
        index += 1;
    }
    index
}

/// Whether `hashes` `#` characters follow at `index`, closing a raw string.
fn closes_raw_string(characters: &[char], index: usize, hashes: usize) -> bool {
    (0..hashes).all(|offset| characters.get(index + offset) == Some(&'#'))
}

/// The unsafe constructs a comment-stripped source text uses.
fn unsafe_constructs(source: &str) -> Vec<&'static str> {
    let characters: Vec<char> = source.chars().collect();
    let mut constructs = Vec::new();
    for start in 0..characters.len() {
        if !keyword_starts_at(&characters, start) {
            continue;
        }
        constructs.extend(construct_after(&characters, start));
    }
    constructs.sort_unstable();
    constructs.dedup();
    constructs
}

/// Whether the `unsafe` keyword starts at `start` as a whole word.
///
/// The spelling check is separate from the boundary checks so that a window of
/// six characters that merely happens to precede `fn` is not classified.
fn keyword_starts_at(characters: &[char], start: usize) -> bool {
    let end = start + UNSAFE_KEYWORD.len();
    characters.get(start..end) == Some(UNSAFE_KEYWORD.as_slice())
        && !is_identifier_character(preceding_character(characters, start))
        && !is_identifier_character(characters.get(end))
}

/// The character before `index`, if there is one.
fn preceding_character(characters: &[char], index: usize) -> Option<&char> {
    index
        .checked_sub(1)
        .and_then(|previous| characters.get(previous))
}

/// Whether a character can continue an identifier.
fn is_identifier_character(character: Option<&char>) -> bool {
    character.is_some_and(|candidate| candidate.is_alphanumeric() || *candidate == '_')
}

/// The construct the `unsafe` keyword at `start` introduces, if any.
fn construct_after(characters: &[char], start: usize) -> Option<&'static str> {
    if is_unsafe_attribute(characters, start) {
        return Some("an unsafe attribute");
    }
    let index = skip_whitespace(characters, start + UNSAFE_KEYWORD.len());
    if characters.get(index) == Some(&'{') {
        return Some("an unsafe block");
    }
    let keyword: String = characters
        .iter()
        .skip(index)
        .take_while(|character| character.is_alphanumeric() || **character == '_')
        .collect();
    construct_for_keyword(&keyword)
}

/// The construct a keyword after `unsafe` names.
fn construct_for_keyword(keyword: &str) -> Option<&'static str> {
    match keyword {
        "fn" => Some("an unsafe function"),
        "trait" => Some("an unsafe trait"),
        "impl" => Some("an unsafe trait implementation"),
        "extern" => Some("an unsafe extern item"),
        _ => None,
    }
}

/// The index of the first non-whitespace character at or after `start`.
fn skip_whitespace(characters: &[char], start: usize) -> usize {
    characters
        .iter()
        .enumerate()
        .skip(start)
        .find(|(_, character)| !character.is_whitespace())
        .map_or(characters.len(), |(index, _)| index)
}

/// Whether the `unsafe` keyword at `start` opens a Rust 2024 unsafe attribute.
fn is_unsafe_attribute(characters: &[char], start: usize) -> bool {
    let significant: Vec<char> = characters
        .iter()
        .take(start)
        .rev()
        .filter(|character| !character.is_whitespace())
        .take(3)
        .copied()
        .collect();
    matches!(significant.as_slice(), ['[', '#'] | ['[', '!', '#'])
}

/// Collapse every run of whitespace to a single space.
///
/// `check-fmt` only formats workspace members, so a source in the standalone
/// `tests/feature_gate` package can be committed with `unsafe` and the rest of
/// a construct on separate lines. Normalizing first means the scan sees the
/// spelling the compiler would, whether the tokens were split across a line
/// break or an attribute list was wrapped by `rustfmt`.
fn collapse_whitespace(source: &str) -> String {
    source.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Whether any `kind` lint attribute in `source` lists `unsafe_code`.
///
/// `kind` is an attribute name with its opening parenthesis, such as `"allow("`.
/// The lint list can name several lints, so `#[allow(dead_code, unsafe_code)]`
/// counts as well as `#[allow(unsafe_code)]`. Each comma-separated element is
/// compared whole, so an `unsafe_code` that appears inside a nested attribute —
/// as in `#![expect(#![forbid(unsafe_code)] dead_code)]` — is not mistaken for
/// a lint this attribute names.
fn lint_attribute_names_unsafe_code(source: &str, kind: &str) -> bool {
    source.match_indices(kind).any(|(index, _)| {
        source
            .get(index + kind.len()..)
            .and_then(|rest| rest.split_once(')'))
            .is_some_and(|(arguments, _)| {
                arguments
                    .split(',')
                    .any(|lint| lint.trim() == "unsafe_code")
            })
    })
}

#[cfg(test)]
#[path = "unsafe_syntax/tests.rs"]
mod tests;
