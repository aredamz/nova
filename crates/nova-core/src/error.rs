use std::fmt;

/// Stable, machine-readable error category (job metadata, API responses).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCode {
    /// The input is not valid JSON or does not match the DSL structure.
    InvalidTemplate,
    /// `schema_version` is not one this build understands.
    UnsupportedVersion,
    /// Structurally valid but semantically wrong (bad reference, bad range...).
    Validation,
    /// A `{{slot}}` has no value in the supplied data.
    MissingData,
    /// Asset missing, unreadable, outside the asset root or undecodable.
    Asset,
    /// A configured resource limit was exceeded.
    Limit,
    Render,
    Encode,
    Io,
    Cancelled,
    Internal,
}

impl ErrorCode {
    pub fn as_str(self) -> &'static str {
        match self {
            ErrorCode::InvalidTemplate => "invalid_template",
            ErrorCode::UnsupportedVersion => "unsupported_version",
            ErrorCode::Validation => "validation",
            ErrorCode::MissingData => "missing_data",
            ErrorCode::Asset => "asset",
            ErrorCode::Limit => "limit",
            ErrorCode::Render => "render",
            ErrorCode::Encode => "encode",
            ErrorCode::Io => "io",
            ErrorCode::Cancelled => "cancelled",
            ErrorCode::Internal => "internal",
        }
    }

    /// Only transient failures are worth retrying; a bad template never is.
    pub fn is_retryable(self) -> bool {
        matches!(self, ErrorCode::Io | ErrorCode::Encode)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Error {
    pub code: ErrorCode,
    pub message: String,
    /// Where in the template the problem is, e.g. `layers[2].animations[0]`.
    pub path: Option<String>,
}

pub type Result<T> = std::result::Result<T, Error>;

impl Error {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Error {
            code,
            message: message.into(),
            path: None,
        }
    }

    pub fn at(code: ErrorCode, path: impl Into<String>, message: impl Into<String>) -> Self {
        Error {
            code,
            message: message.into(),
            path: Some(path.into()),
        }
    }

    pub fn validation(path: impl Into<String>, message: impl Into<String>) -> Self {
        Error::at(ErrorCode::Validation, path, message)
    }

    pub fn io(context: impl fmt::Display, err: std::io::Error) -> Self {
        Error::new(ErrorCode::Io, format!("{context}: {err}"))
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.path {
            Some(p) => write!(f, "[{}] {}: {}", self.code.as_str(), p, self.message),
            None => write!(f, "[{}] {}", self.code.as_str(), self.message),
        }
    }
}

impl std::error::Error for Error {}
