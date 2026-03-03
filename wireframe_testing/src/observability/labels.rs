//! Owned metric label pairs for counter queries.

/// Owned metric label pairs for use with counter queries.
///
/// Provides a builder API and conversions from `&[(&str, &str)]` so
/// callers can use either literal slices or the builder pattern.
///
/// # Examples
///
/// ```no_run
/// use wireframe_testing::observability::Labels;
///
/// // Builder pattern
/// let labels = Labels::new()
///     .with("error_type", "framing")
///     .with("recovery_policy", "drop");
///
/// // From a literal array
/// let labels: Labels = [("error_type", "framing"), ("recovery_policy", "drop")].into();
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Labels(Vec<(String, String)>);

impl Labels {
    /// Create an empty label set.
    #[must_use]
    pub fn new() -> Self { Self::default() }

    /// Append a key-value pair, returning `self` for chaining.
    #[must_use]
    pub fn with(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.0.push((key.into(), value.into()));
        self
    }

    /// Convert to temporary borrowed pairs for internal queries.
    pub(crate) fn as_str_pairs(&self) -> Vec<(&str, &str)> {
        self.0
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect()
    }
}

impl From<&[(&str, &str)]> for Labels {
    fn from(pairs: &[(&str, &str)]) -> Self {
        Self(
            pairs
                .iter()
                .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
                .collect(),
        )
    }
}

impl<const N: usize> From<[(&str, &str); N]> for Labels {
    fn from(pairs: [(&str, &str); N]) -> Self {
        Self(
            pairs
                .into_iter()
                .map(|(k, v)| (k.to_owned(), v.to_owned()))
                .collect(),
        )
    }
}

impl<const N: usize> From<&[(&str, &str); N]> for Labels {
    fn from(pairs: &[(&str, &str); N]) -> Self {
        Self(
            pairs
                .iter()
                .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
                .collect(),
        )
    }
}
