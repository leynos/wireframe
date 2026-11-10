//! Fixture world for published-document example scenarios.

use std::collections::HashSet;

use rstest::fixture;

use crate::tested_examples::{
    PublishedDocument,
    TestedExample,
    load_published_examples,
    parse_document,
};

/// Fallible result returned by documentation-example scenario checks.
pub type TestResult<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

/// A document whose marker and fence satisfy every loader rule.
pub const VALID_DOCUMENT: &str = concat!(
    "<!-- tested-example: staged-001 -->\n",
    "```rust\n",
    "fn main() {}\n",
    "```\n",
);

/// A document whose fenced example names no language, so the loader rejects it.
pub const UNLANGUAGED_DOCUMENT: &str = concat!(
    "<!-- tested-example: staged-001 -->\n",
    "```\n",
    "fn main() {}\n",
    "```\n",
);

/// BDD world holding examples extracted from published Markdown documents.
///
/// The world either reports on the published inventory or validates a staged
/// document, mirroring the two ways the shared loader is used: proving that
/// shipped examples load, and rejecting a document that breaks the marker
/// rules.
#[derive(Debug, Default)]
pub struct TestedExamplesWorld {
    examples: Option<Vec<TestedExample>>,
    staged_document: Option<String>,
    validation: Option<Result<(), String>>,
}

#[rustfmt::skip]
#[fixture]
pub fn tested_examples_world() -> TestedExamplesWorld {
    std::hint::black_box(());
    TestedExamplesWorld::default()
}

impl TestedExamplesWorld {
    /// Load user-facing examples through the shared Markdown loader.
    pub fn load(&mut self) -> TestResult<()> {
        self.examples = Some(load_published_examples().map_err(std::io::Error::other)?);
        Ok(())
    }

    /// Verify each loaded example has a usable, unique identifier and source.
    pub fn verify_examples_are_addressable(&self) -> TestResult<()> {
        let examples = self
            .examples
            .as_ref()
            .ok_or_else(|| std::io::Error::other("published examples were not loaded"))?;
        let identifiers: HashSet<&str> =
            examples.iter().map(|example| example.id.as_str()).collect();
        if examples.is_empty() || identifiers.len() != examples.len() {
            return Err(std::io::Error::other(
                "published examples are empty or have duplicate identifiers",
            )
            .into());
        }
        if examples
            .iter()
            .any(|example| example.source.trim().is_empty() || example.language.is_empty())
        {
            return Err(
                std::io::Error::other("a published example has no language or source").into(),
            );
        }
        Ok(())
    }

    /// Stage a Markdown document for the validation scenarios.
    pub fn stage_document(&mut self, source: &str) {
        self.staged_document = Some(source.to_owned());
        self.validation = None;
    }

    /// Validate the staged document through the same loader the tests use.
    pub fn validate_staged_document(&mut self) -> TestResult<()> {
        let source = self
            .staged_document
            .as_deref()
            .ok_or_else(|| std::io::Error::other("no document was staged"))?;
        let document = PublishedDocument::new("staged.md", source);
        self.validation = Some(parse_document(&document, &HashSet::new()).map(|_| ()));
        Ok(())
    }

    /// Verify validation accepted the staged document.
    pub fn validation_succeeded(&self) -> TestResult<()> {
        match self.validation.as_ref() {
            Some(Ok(())) => Ok(()),
            Some(Err(message)) => Err(std::io::Error::other(format!(
                "validation failed unexpectedly: {message}"
            ))
            .into()),
            None => Err(std::io::Error::other("the document was not validated").into()),
        }
    }

    /// Verify validation rejected the staged document for the stated reason.
    pub fn validation_failed_with(&self, fragment: &str) -> TestResult<()> {
        match self.validation.as_ref() {
            Some(Err(message)) if message.contains(fragment) => Ok(()),
            Some(Err(message)) => Err(std::io::Error::other(format!(
                "validation failed for an unexpected reason: {message}"
            ))
            .into()),
            Some(Ok(())) => Err(std::io::Error::other("validation unexpectedly passed").into()),
            None => Err(std::io::Error::other("the document was not validated").into()),
        }
    }
}
