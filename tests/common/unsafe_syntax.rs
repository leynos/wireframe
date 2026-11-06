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

/// A position in a source text's character buffer.
///
/// Positions advance and compare as a unit, so a scan cannot mix an index into
/// one buffer with an offset meant for another.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Position(usize);

impl Position {
    /// The position of the first character.
    const START: Self = Self(0);

    /// The buffer index this position names.
    fn index(self) -> usize { self.0 }

    /// The position one character later.
    fn next(self) -> Self { Self(self.0 + 1) }

    /// The position `count` characters later.
    fn advance(self, count: usize) -> Self { Self(self.0 + count) }
}

/// A source text's characters, scanned as a unit.
struct SourceText {
    characters: Vec<char>,
}

impl SourceText {
    /// Collect `source` into a scannable buffer.
    fn new(source: &str) -> Self {
        Self {
            characters: source.chars().collect(),
        }
    }

    /// The character at `position`, if the buffer reaches it.
    fn character_at(&self, position: Position) -> Option<&char> {
        self.characters.get(position.index())
    }

    /// The position just past the last character.
    fn end(&self) -> Position { Position(self.characters.len()) }

    /// Replace every Rust comment with a single space.
    ///
    /// Rust treats a comment as whitespace, so `unsafe/* SAFETY */fn` is an unsafe
    /// function and must be reported. String, character, and raw-string literals
    /// are copied verbatim: a literal such as `"src/**/*.rs"` must not start a
    /// block comment that would swallow the code after it.
    fn strip_comments(&self) -> String {
        let mut stripped = String::with_capacity(self.characters.len());
        let mut position = Position::START;
        while let Some(character) = self.character_at(position) {
            position = self.strip_next(position, *character, &mut stripped);
        }
        stripped
    }

    /// Copy or skip the token that starts at `start`, returning the next position.
    fn strip_next(&self, start: Position, character: char, output: &mut String) -> Position {
        match character {
            '/' if self.is_line_comment_start(start) => {
                output.push(' ');
                self.skip_line_comment(start)
            }
            '/' if self.is_block_comment_opener(start) => {
                output.push(' ');
                self.skip_block_comment(start)
            }
            '"' => self.copy_quoted(start, '"', output),
            '\'' => self.copy_character_literal(start, output),
            'r' if self.raw_string_hashes(start).is_some() => self.copy_raw_string(start, output),
            _ => {
                output.push(character);
                start.next()
            }
        }
    }

    /// Whether a line comment starts at `position`.
    fn is_line_comment_start(&self, position: Position) -> bool {
        self.character_at(position) == Some(&'/')
            && self.character_at(position.next()) == Some(&'/')
    }

    /// Return the position of the line break that ends the comment at `start`.
    fn skip_line_comment(&self, start: Position) -> Position {
        self.characters
            .iter()
            .enumerate()
            .skip(start.index())
            .find(|(_, character)| **character == '\n')
            .map_or_else(|| self.end(), |(index, _)| Position(index))
    }

    /// Return the position after the block comment that starts at `start`.
    fn skip_block_comment(&self, start: Position) -> Position {
        let mut depth = 1;
        let mut position = start.advance(2);
        while self.character_at(position).is_some() {
            let (next, change) = self.block_comment_step(position);
            depth += change;
            position = next;
            if depth == 0 {
                return position;
            }
        }
        self.end()
    }

    /// The position after the comment token at `position`, and its depth change.
    ///
    /// Rust nests block comments, so an opener or closer advances two characters
    /// and changes the depth by one; any other character advances one and changes
    /// nothing.
    fn block_comment_step(&self, position: Position) -> (Position, i32) {
        if self.is_block_comment_opener(position) {
            (position.advance(2), 1)
        } else if self.is_block_comment_closer(position) {
            (position.advance(2), -1)
        } else {
            (position.next(), 0)
        }
    }

    /// Whether `position` opens a block comment.
    fn is_block_comment_opener(&self, position: Position) -> bool {
        self.character_at(position) == Some(&'/')
            && self.character_at(position.next()) == Some(&'*')
    }

    /// Whether `position` closes a block comment.
    fn is_block_comment_closer(&self, position: Position) -> bool {
        self.character_at(position) == Some(&'*')
            && self.character_at(position.next()) == Some(&'/')
    }

    /// The unsafe constructs this comment-stripped text uses.
    fn unsafe_constructs(&self) -> Vec<&'static str> {
        let mut constructs = Vec::new();
        for index in 0..self.characters.len() {
            let start = Position(index);
            if !self.keyword_starts_at(start) {
                continue;
            }
            constructs.extend(self.construct_after(start));
        }
        constructs.sort_unstable();
        constructs.dedup();
        constructs
    }

    /// Whether the `unsafe` keyword starts at `start` as a whole word.
    ///
    /// The spelling check is separate from the boundary checks so that a window of
    /// six characters that merely happens to precede `fn` is not classified.
    fn keyword_starts_at(&self, start: Position) -> bool {
        let end = start.advance(UNSAFE_KEYWORD.len());
        self.characters.get(start.index()..end.index()) == Some(UNSAFE_KEYWORD.as_slice())
            && !is_identifier_character(self.preceding_character(start))
            && !is_identifier_character(self.character_at(end))
    }

    /// The character before `position`, if there is one.
    fn preceding_character(&self, position: Position) -> Option<&char> {
        position
            .index()
            .checked_sub(1)
            .and_then(|previous| self.characters.get(previous))
    }

    /// The construct the `unsafe` keyword at `start` introduces, if any.
    fn construct_after(&self, start: Position) -> Option<&'static str> {
        if self.is_unsafe_attribute(start) {
            return Some("an unsafe attribute");
        }
        let position = self.skip_whitespace(start.advance(UNSAFE_KEYWORD.len()));
        if self.character_at(position) == Some(&'{') {
            return Some("an unsafe block");
        }
        let keyword: String = self
            .characters
            .iter()
            .skip(position.index())
            .take_while(|character| character.is_alphanumeric() || **character == '_')
            .collect();
        Keyword(keyword).construct()
    }

    /// The position of the first non-whitespace character at or after `start`.
    fn skip_whitespace(&self, start: Position) -> Position {
        self.characters
            .iter()
            .enumerate()
            .skip(start.index())
            .find(|(_, character)| !character.is_whitespace())
            .map_or_else(|| self.end(), |(index, _)| Position(index))
    }

    /// Whether the `unsafe` keyword at `start` opens a Rust 2024 unsafe attribute.
    fn is_unsafe_attribute(&self, start: Position) -> bool {
        let significant: Vec<char> = self
            .characters
            .iter()
            .take(start.index())
            .rev()
            .filter(|character| !character.is_whitespace())
            .take(3)
            .copied()
            .collect();
        matches!(significant.as_slice(), ['[', '#'] | ['[', '!', '#'])
    }
}

/// The identifier that follows the `unsafe` keyword.
struct Keyword(String);

impl Keyword {
    /// The construct this keyword names, if the scan reports it.
    fn construct(&self) -> Option<&'static str> {
        match self.0.as_str() {
            "fn" => Some("an unsafe function"),
            "trait" => Some("an unsafe trait"),
            "impl" => Some("an unsafe trait implementation"),
            "extern" => Some("an unsafe extern item"),
            _ => None,
        }
    }
}

/// A lint attribute whose arguments are searched for `unsafe_code`.
#[derive(Clone, Copy)]
enum LintAttribute {
    /// `#[allow(...)]`.
    Allow,
    /// `#[expect(...)]`.
    Expect,
}

impl LintAttribute {
    /// The attribute name and opening parenthesis this kind is spelled with.
    fn opening(self) -> &'static str {
        match self {
            Self::Allow => "allow(",
            Self::Expect => "expect(",
        }
    }
}

/// A comment-stripped source with its whitespace collapsed.
struct NormalizedSource(String);

impl NormalizedSource {
    /// Collapse every run of whitespace in `source` to a single space.
    ///
    /// `check-fmt` only formats workspace members, so a source in the standalone
    /// `tests/feature_gate` package can be committed with `unsafe` and the rest of
    /// a construct on separate lines. Normalizing first means the scan sees the
    /// spelling the compiler would, whether the tokens were split across a line
    /// break or an attribute list was wrapped by `rustfmt`.
    fn new(source: &str) -> Self { Self(source.split_whitespace().collect::<Vec<_>>().join(" ")) }

    /// Whether any `attribute` in this source lists `unsafe_code`.
    ///
    /// The lint list can name several lints, so `#[allow(dead_code, unsafe_code)]`
    /// counts as well as `#[allow(unsafe_code)]`. Each comma-separated element is
    /// compared whole, so an `unsafe_code` that appears inside a nested attribute —
    /// as in `#![expect(#![forbid(unsafe_code)] dead_code)]` — is not mistaken for
    /// a lint this attribute names.
    fn names_unsafe_code(&self, attribute: LintAttribute) -> bool {
        let kind = attribute.opening();
        self.0.match_indices(kind).any(|(index, _)| {
            self.0
                .get(index + kind.len()..)
                .and_then(|rest| rest.split_once(')'))
                .is_some_and(|(arguments, _)| {
                    arguments
                        .split(',')
                        .any(|lint| lint.trim() == "unsafe_code")
                })
        })
    }
}

/// Whether a character can continue an identifier.
fn is_identifier_character(character: Option<&char>) -> bool {
    character.is_some_and(|candidate| candidate.is_alphanumeric() || *candidate == '_')
}

/// Unsafe-code violations in one source text.
///
/// An `unsafe_code` allowance is always a violation: `forbid` admits no
/// override, so the allowance cannot take effect and its presence means a root
/// is trying to weaken the prohibition. A scoped expectation and an unsafe
/// construct are violations too, because the workspace contains no sanctioned
/// unsafe code.
pub(crate) fn violations_in(source: &str, label: &str) -> Vec<String> {
    let code = SourceText::new(source).strip_comments();
    let normalized_source = NormalizedSource::new(&code);
    let mut violations = Vec::new();
    if normalized_source.names_unsafe_code(LintAttribute::Allow) {
        violations.push(format!(
            "{label} uses an `unsafe_code` allowance; the workspace forbids unsafe code"
        ));
    }
    if normalized_source.names_unsafe_code(LintAttribute::Expect) {
        violations.push(format!(
            "{label} carries an `unsafe_code` expectation; the workspace forbids unsafe code"
        ));
    }
    violations.extend(
        SourceText::new(&code)
            .unsafe_constructs()
            .into_iter()
            .map(|construct| {
                format!("{label} contains {construct} but the workspace forbids unsafe code")
            }),
    );
    violations
}

#[path = "unsafe_syntax/literals.rs"]
mod literals;

#[cfg(test)]
#[path = "unsafe_syntax/tests.rs"]
mod tests;
