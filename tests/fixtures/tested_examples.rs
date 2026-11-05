//! Fixture world for published-document example scenarios.

use rstest::fixture;

use crate::tested_examples::{TestedExample, load_published_examples};

/// Fallible result returned by documentation-example scenario checks.
pub type TestResult<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

/// BDD world holding examples extracted from published Markdown documents.
#[derive(Debug, Default)]
pub struct TestedExamplesWorld {
    examples: Option<Vec<TestedExample>>,
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
        let identifiers: std::collections::HashSet<&str> =
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
}
