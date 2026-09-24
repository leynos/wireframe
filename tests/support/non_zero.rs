/// Turn a fixture budget into a constant `NonZeroUsize`, rejecting zero at
/// compile time. The optional message identifies the fixture that failed.
macro_rules! nz {
    ($value:expr) => {
        nz!($value, "test budget must be non-zero")
    };
    ($value:expr, $message:literal) => {{
        const NON_ZERO: ::std::num::NonZeroUsize = match ::std::num::NonZeroUsize::new($value) {
            Some(value) => value,
            None => panic!($message),
        };
        NON_ZERO
    }};
}
