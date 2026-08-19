//! Error types for `proxi`.

use std::ffi::NulError;

/// Errors originating in PROJ or from `proxi` validation.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ProxiError {
    /// A CRS definition could not be created from user input.
    #[error("invalid CRS {input:?}: {message}")]
    InvalidCrs {
        /// The input string that failed.
        input: String,
        /// PROJ's error message.
        message: String,
    },
    /// A coordinate operation (transformer) could not be created.
    #[error("invalid transformer ({source_crs:?} -> {target_crs:?}): {message}")]
    InvalidTransformer {
        /// The source CRS definition.
        source_crs: String,
        /// The target CRS definition.
        target_crs: String,
        /// PROJ's error message.
        message: String,
    },
    /// An error occurred while transforming coordinates.
    #[error("transform failed (PROJ error {code}): {message}")]
    Transform {
        /// PROJ error code.
        code: i32,
        /// Human-readable message.
        message: String,
    },
    #[error("required transformation grid is unavailable: {message}")]
    GridMissing { message: String },
    /// The supplied buffers/layouts have mismatched lengths.
    #[error("length mismatch: buffer `{name}` has length {actual}, expected {expected}")]
    LengthMismatch {
        /// The name of the offending coordinate buffer.
        name: &'static str,
        /// The expected (reference) length.
        expected: usize,
        /// The actual length of the buffer.
        actual: usize,
    },
    /// Required PROJ runtime data (e.g. `proj.db`) is missing.
    #[error("missing PROJ data: {message}")]
    MissingData {
        /// Diagnostic message, including active search paths when known.
        message: String,
    },
    /// PROJ context configuration failed.
    #[error("context configuration failed: {message}")]
    ContextConfiguration {
        /// Diagnostic message from the failed context operation.
        message: String,
    },
    /// The loaded PROJ runtime is incompatible with the generated bindings.
    #[error(
        "incompatible PROJ runtime: built {built_major}.{built_minor}.{built_patch}, runtime {runtime_major}.{runtime_minor}.{runtime_patch}"
    )]
    VersionMismatch {
        built_major: u32,
        built_minor: u32,
        built_patch: u32,
        runtime_major: u32,
        runtime_minor: u32,
        runtime_patch: u32,
    },
    /// A requested capability is unavailable in the selected native backend.
    #[error("unsupported capability: {feature}")]
    Unsupported { feature: &'static str },
    /// An interior NUL byte was found in a string passed to PROJ.
    #[error("string contains interior NUL byte: {0}")]
    Nul(#[from] NulError),
}

/// Result of a transform that may have partially failed.
///
/// Default-policy partial failures carry only the processed/total summary (no
/// per-point indices — computing them is opt-in via `verify_points` and has a
/// documented performance cost, per the design correction #6).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PartialFailure {
    /// Number of coordinates successfully transformed (PROJ committed these).
    pub processed: usize,
    /// Total number of coordinates in the batch.
    pub total: usize,
}

impl PartialFailure {
    /// Number of coordinates that were not transformed (`total - processed`).
    pub fn failed(&self) -> usize {
        self.total - self.processed
    }
}

/// Convenience result alias used throughout `proxi`.
pub type Result<T> = std::result::Result<T, ProxiError>;
