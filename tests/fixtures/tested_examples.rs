//! Fixture world for published-document example scenarios.
//!
//! The world drives the same two surfaces a contributor does: the published
//! inventory that `make test` loads, and the loader gate that rejects a shipped
//! document whose markers no longer pair with its fences. The malformed case
//! mutates a real guide read from disk, so the scenario covers the file input
//! and the merge boundary rather than a test-only parser helper.

use std::collections::HashSet;

use rstest::fixture;

use crate::{
    repo_access::read_repo_file,
    tested_examples::{TestedExample, document_paths, load_documents, load_published_examples},
};

/// Fallible result returned by documentation-example scenario checks.
pub type TestResult<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

/// Marker removed to make a shipped guide fail the loader gate.
pub const REMOVED_MARKER: &str = "<!-- tested-example: users-guide-001 -->\n";

/// The shipped document whose marker the malformed scenario removes.
pub const SHIPPED_DOCUMENT: &str = "docs/users-guide.md";

/// BDD world holding the documentation gate's prepared input and outcome.
///
/// A scenario reads the published documents from disk, optionally breaking one
/// of them, and then runs the shared loader over the prepared set. `validation`
/// records what the gate returned; the `Then` steps inspect it.
#[derive(Debug, Default)]
pub struct TestedExamplesWorld {
    examples: Option<Vec<TestedExample>>,
    documents: Option<Vec<(String, String)>>,
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

    /// Read every shipped document from disk, ready for the loader gate.
    pub fn prepare_shipped_documents(&mut self) -> TestResult<()> {
        self.documents = Some(read_shipped_documents()?);
        self.validation = None;
        Ok(())
    }

    /// Read the shipped documents, then strip one marker from one of them.
    pub fn prepare_shipped_documents_missing_marker(&mut self) -> TestResult<()> {
        let mut documents = read_shipped_documents()?;
        let (_, source) = documents
            .iter_mut()
            .find(|(path, _)| path == SHIPPED_DOCUMENT)
            .ok_or_else(|| std::io::Error::other("the shipped guide is not in the inventory"))?;
        *source = source.replacen(REMOVED_MARKER, "", 1);
        self.documents = Some(documents);
        self.validation = None;
        Ok(())
    }

    /// Run the shared loader's merge boundary over the prepared documents.
    pub fn validate_prepared_documents(&mut self) -> TestResult<()> {
        let documents = self
            .documents
            .as_ref()
            .ok_or_else(|| std::io::Error::other("no documents were prepared"))?;
        let pairs: Vec<(&str, &str)> = documents
            .iter()
            .map(|(path, source)| (path.as_str(), source.as_str()))
            .collect();
        self.validation = Some(load_documents(&pairs).map(|_| ()));
        Ok(())
    }

    /// Verify the gate accepted the prepared documents.
    pub fn validation_succeeded(&self) -> TestResult<()> {
        match self.validation.as_ref() {
            Some(Ok(())) => Ok(()),
            Some(Err(message)) => Err(std::io::Error::other(format!(
                "validation failed unexpectedly: {message}"
            ))
            .into()),
            None => Err(std::io::Error::other("the document set was not validated").into()),
        }
    }

    /// Verify the gate rejected the prepared documents for the stated reasons.
    pub fn validation_failed_with_all(&self, fragments: &[&str]) -> TestResult<()> {
        match self.validation.as_ref() {
            Some(Err(message)) => fragments
                .iter()
                .find(|fragment| !message.contains(**fragment))
                .map_or_else(
                    || Ok(()),
                    |missing| {
                        Err(std::io::Error::other(format!(
                            "validation did not report `{missing}`: {message}"
                        ))
                        .into())
                    },
                ),
            Some(Ok(())) => Err(std::io::Error::other("validation unexpectedly passed").into()),
            None => Err(std::io::Error::other("the document set was not validated").into()),
        }
    }
}

/// Read the shipped document inventory from disk.
fn read_shipped_documents() -> TestResult<Vec<(String, String)>> {
    document_paths()
        .map(|path| {
            let source = read_repo_file(path)?;
            Ok((path.to_owned(), source))
        })
        .collect()
}
