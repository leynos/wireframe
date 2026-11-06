//! Copy the literals a comment-stripping scan must pass through untouched.
//!
//! A `//` or `/*` inside a string, character, or raw-string literal is data,
//! not a comment. The scan copies each literal verbatim so the code after it is
//! still scanned, and so a literal that spells a prohibited construct out is
//! reported rather than skipped.

use super::{Position, SourceText};

/// The number of `#` characters delimiting a raw string.
#[derive(Clone, Copy)]
pub(super) struct HashCount(usize);

impl SourceText {
    /// Copy the literal delimited by `quote` that starts at `start`.
    pub(super) fn copy_quoted(
        &self,
        start: Position,
        quote: char,
        output: &mut String,
    ) -> Position {
        output.push(quote);
        let mut position = start.next();
        while let Some(character) = self.character_at(position) {
            output.push(*character);
            position = position.next();
            if *character == '\\' {
                position = self.copy_escaped(position, output);
            } else if *character == quote {
                break;
            }
        }
        position
    }

    /// Copy the character escaped by a backslash at `position`, returning the next one.
    fn copy_escaped(&self, position: Position, output: &mut String) -> Position {
        let Some(escaped) = self.character_at(position) else {
            return position;
        };
        output.push(*escaped);
        position.next()
    }

    /// Copy a character literal, or just the apostrophe of a lifetime.
    pub(super) fn copy_character_literal(&self, start: Position, output: &mut String) -> Position {
        if self.is_character_literal(start) {
            self.copy_quoted(start, '\'', output)
        } else {
            output.push('\'');
            start.next()
        }
    }

    /// Whether the apostrophe at `start` opens a character literal.
    fn is_character_literal(&self, start: Position) -> bool {
        match self.character_at(start.next()) {
            Some('\\') => true,
            Some(_) => self.character_at(start.advance(2)) == Some(&'\''),
            None => false,
        }
    }

    /// The number of `#` characters in a raw-string opener at `start`, if any.
    pub(super) fn raw_string_hashes(&self, start: Position) -> Option<HashCount> {
        if self.character_at(start) != Some(&'r') {
            return None;
        }
        let hashes = self
            .characters
            .iter()
            .skip(start.next().index())
            .take_while(|character| **character == '#')
            .count();
        (self.character_at(start.advance(1 + hashes)) == Some(&'"')).then_some(HashCount(hashes))
    }

    /// Copy the raw string that starts at `start`.
    pub(super) fn copy_raw_string(&self, start: Position, output: &mut String) -> Position {
        let hashes = self.raw_string_hashes(start).unwrap_or(HashCount(0));
        let opening_quote = start.advance(1 + hashes.0);
        let mut position = opening_quote.next();
        let opener_length = position.index() - start.index();
        for character in self
            .characters
            .iter()
            .skip(start.index())
            .take(opener_length)
        {
            output.push(*character);
        }
        while let Some(character) = self.character_at(position) {
            output.push(*character);
            position = position.next();
            if *character == '"' && self.closes_raw_string(position, hashes) {
                return self.copy_closing_hashes(position, hashes, output);
            }
        }
        position
    }

    /// Copy the `hashes` closing `#` characters at `position`, returning the next one.
    fn copy_closing_hashes(
        &self,
        mut position: Position,
        hashes: HashCount,
        output: &mut String,
    ) -> Position {
        for _ in 0..hashes.0 {
            let Some(hash) = self.character_at(position) else {
                break;
            };
            output.push(*hash);
            position = position.next();
        }
        position
    }

    /// Whether `hashes` `#` characters follow at `position`, closing a raw string.
    fn closes_raw_string(&self, position: Position, hashes: HashCount) -> bool {
        (0..hashes.0).all(|offset| self.character_at(position.advance(offset)) == Some(&'#'))
    }
}
