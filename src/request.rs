//! Request metadata and streaming body types for handler consumption.
//!
//! These types allow handlers to consume request payloads incrementally
//! rather than waiting for full reassembly. See ADR 0002 for the design
//! rationale and composition with transport-level fragmentation.

/// Request metadata extracted outwith the request body.
///
/// `RequestParts` separates routing and protocol metadata from the request
/// payload, enabling handlers to begin processing before the full body
/// arrives. This struct is the counterpart to streaming response types
/// ([`crate::Response::Stream`]) for the inbound direction.
///
/// # Differences from [`crate::app::PacketParts`]
///
/// - [`crate::app::PacketParts`] carries the raw `payload` for packet routing and envelope
///   reconstruction.
/// - `RequestParts` carries protocol-defined `metadata` bytes (for example headers) that handlers
///   need to interpret the body, but excludes the body itself.
///
/// # Examples
///
/// ```
/// use wireframe::request::RequestParts;
///
/// let parts = RequestParts::new(42, Some(123), vec![0x01, 0x02]);
/// assert_eq!(parts.id(), 42);
/// assert_eq!(parts.correlation_id(), Some(123));
/// assert_eq!(parts.metadata(), &[0x01, 0x02]);
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestParts {
    /// Protocol-specific packet or message identifier used for routing.
    id: u32,
    /// Correlation identifier, if present.
    correlation_id: Option<u64>,
    /// Protocol-defined metadata bytes (for example headers) required to
    /// interpret the body.
    metadata: Vec<u8>,
}

impl RequestParts {
    /// Construct a new set of request parts.
    ///
    /// # Examples
    ///
    /// ```
    /// use wireframe::request::RequestParts;
    ///
    /// let parts = RequestParts::new(1, None, vec![0xab, 0xcd]);
    /// assert_eq!(parts.id(), 1);
    /// ```
    #[must_use]
    pub fn new(id: u32, correlation_id: Option<u64>, metadata: Vec<u8>) -> Self {
        Self {
            id,
            correlation_id,
            metadata,
        }
    }

    /// Return the message identifier used to route this request.
    ///
    /// # Examples
    ///
    /// ```
    /// use wireframe::request::RequestParts;
    ///
    /// let parts = RequestParts::new(9, None, vec![]);
    /// assert_eq!(parts.id(), 9);
    /// ```
    #[must_use]
    pub const fn id(&self) -> u32 { self.id }

    /// Retrieve the correlation identifier, if present.
    ///
    /// The correlation identifier ties this request to a logical session
    /// or prior exchange when the protocol supports request chaining.
    ///
    /// # Examples
    ///
    /// ```
    /// use wireframe::request::RequestParts;
    ///
    /// let parts = RequestParts::new(1, Some(42), vec![]);
    /// assert_eq!(parts.correlation_id(), Some(42));
    /// ```
    #[must_use]
    pub const fn correlation_id(&self) -> Option<u64> { self.correlation_id }

    /// Return a reference to the protocol-defined metadata bytes.
    ///
    /// Metadata typically contains protocol headers, content-type markers,
    /// or other information required to interpret the streaming body.
    ///
    /// # Examples
    ///
    /// ```
    /// use wireframe::request::RequestParts;
    ///
    /// let parts = RequestParts::new(1, None, vec![0x01, 0x02, 0x03]);
    /// assert_eq!(parts.metadata(), &[0x01, 0x02, 0x03]);
    /// ```
    #[must_use]
    pub fn metadata(&self) -> &[u8] { &self.metadata }

    /// Return a mutable reference to the protocol-defined metadata bytes.
    ///
    /// # Examples
    ///
    /// ```
    /// use wireframe::request::RequestParts;
    ///
    /// let mut parts = RequestParts::new(1, None, vec![0x01]);
    /// parts.metadata_mut().push(0x02);
    /// assert_eq!(parts.metadata(), &[0x01, 0x02]);
    /// ```
    pub fn metadata_mut(&mut self) -> &mut Vec<u8> { &mut self.metadata }

    /// Consume the parts and return the metadata bytes.
    ///
    /// Use this when ownership of the metadata is required without
    /// additional allocation.
    ///
    /// # Examples
    ///
    /// ```
    /// use wireframe::request::RequestParts;
    ///
    /// let parts = RequestParts::new(1, None, vec![7, 8]);
    /// let metadata = parts.into_metadata();
    /// assert_eq!(metadata, vec![7, 8]);
    /// ```
    #[must_use]
    pub fn into_metadata(self) -> Vec<u8> { self.metadata }

    /// Ensure a correlation identifier is present, inheriting from `source`
    /// if missing.
    ///
    /// This mirrors [`crate::app::PacketParts::inherit_correlation`] to ensure
    /// consistent correlation handling across request and response paths.
    ///
    /// # Examples
    ///
    /// ```
    /// use wireframe::request::RequestParts;
    ///
    /// // Inherit when missing
    /// let parts = RequestParts::new(1, None, vec![]).inherit_correlation(Some(42));
    /// assert_eq!(parts.correlation_id(), Some(42));
    ///
    /// // Overwrite mismatched value
    /// let parts = RequestParts::new(1, Some(7), vec![]).inherit_correlation(Some(8));
    /// assert_eq!(parts.correlation_id(), Some(8));
    /// ```
    #[must_use]
    pub fn inherit_correlation(mut self, source: Option<u64>) -> Self {
        let (next, mismatched) = Self::select_correlation(self.correlation_id, source);
        if mismatched && let (Some(found), Some(expected)) = (self.correlation_id, next) {
            log::warn!(
                "mismatched correlation id in request: id={}, expected={}, found={}",
                self.id,
                expected,
                found
            );
        }
        self.correlation_id = next;
        self
    }

    #[inline]
    fn select_correlation(current: Option<u64>, source: Option<u64>) -> (Option<u64>, bool) {
        match (current, source) {
            (None, cid) => (cid, false),
            (Some(cid), Some(src)) if cid != src => (Some(src), true),
            (curr, _) => (curr, false),
        }
    }
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    #[rstest]
    fn new_stores_all_fields() {
        let parts = RequestParts::new(42, Some(100), vec![0x01, 0x02]);
        assert_eq!(parts.id(), 42);
        assert_eq!(parts.correlation_id(), Some(100));
        assert_eq!(parts.metadata(), &[0x01, 0x02]);
    }

    #[rstest]
    fn metadata_returns_reference() {
        let parts = RequestParts::new(1, None, vec![0xab, 0xcd, 0xef]);
        let meta = parts.metadata();
        assert_eq!(meta.len(), 3);
        assert_eq!(meta.first(), Some(&0xab));
    }

    #[rstest]
    fn metadata_mut_allows_modification() {
        let mut parts = RequestParts::new(1, None, vec![0x01]);
        parts.metadata_mut().push(0x02);
        assert_eq!(parts.metadata(), &[0x01, 0x02]);
    }

    #[rstest]
    fn into_metadata_transfers_ownership() {
        let parts = RequestParts::new(1, None, vec![7, 8, 9]);
        let owned = parts.into_metadata();
        assert_eq!(owned, vec![7, 8, 9]);
    }

    #[rstest]
    fn clone_produces_equal_instance() {
        let parts = RequestParts::new(42, Some(100), vec![0x01, 0x02]);
        let cloned = parts.clone();
        assert_eq!(parts, cloned);
    }

    #[rstest]
    #[case(RequestParts::new(1, None, vec![]), Some(42), Some(42))]
    #[case(RequestParts::new(1, Some(7), vec![]), None, Some(7))]
    #[case(RequestParts::new(1, None, vec![]), None, None)]
    #[case(RequestParts::new(1, Some(7), vec![]), Some(8), Some(8))]
    fn inherit_correlation_variants(
        #[case] start: RequestParts,
        #[case] source: Option<u64>,
        #[case] expected: Option<u64>,
    ) {
        let got = start.inherit_correlation(source);
        assert_eq!(got.correlation_id(), expected);
    }

    #[rstest]
    fn empty_metadata_is_valid() {
        let parts = RequestParts::new(1, None, vec![]);
        assert!(parts.metadata().is_empty());
    }
}
