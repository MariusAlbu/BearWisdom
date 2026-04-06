use std::fmt;

/// Structured error type for the query layer.
///
/// Callers can match on the variant to decide whether to retry, show a
/// user-friendly message, or propagate as internal.
#[derive(Debug)]
pub enum QueryError {
    /// The database has not been indexed yet (no files table, empty index).
    NotIndexed,
    /// The requested symbol/file was not found in the index.
    NotFound(String),
    /// SQLite returned SQLITE_BUSY — another writer holds the lock.
    DatabaseBusy,
    /// Any other internal error (schema mismatch, I/O, etc.).
    Internal(anyhow::Error),
}

impl fmt::Display for QueryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotIndexed => write!(f, "Database has not been indexed"),
            Self::NotFound(name) => write!(f, "Not found: {name}"),
            Self::DatabaseBusy => write!(f, "Database is busy (another writer holds the lock)"),
            Self::Internal(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for QueryError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Internal(e) => Some(e.as_ref()),
            _ => None,
        }
    }
}

impl From<anyhow::Error> for QueryError {
    fn from(e: anyhow::Error) -> Self {
        if let Some(rusqlite_err) = e.downcast_ref::<rusqlite::Error>() {
            if matches!(rusqlite_err, rusqlite::Error::SqliteFailure(ffi_err, _)
                if ffi_err.code == rusqlite::ffi::ErrorCode::DatabaseBusy)
            {
                return Self::DatabaseBusy;
            }
        }
        Self::Internal(e)
    }
}

impl From<rusqlite::Error> for QueryError {
    fn from(e: rusqlite::Error) -> Self {
        if matches!(&e, rusqlite::Error::SqliteFailure(ffi_err, _)
            if ffi_err.code == rusqlite::ffi::ErrorCode::DatabaseBusy)
        {
            return Self::DatabaseBusy;
        }
        Self::Internal(e.into())
    }
}

/// Convenience alias used throughout the query layer.
pub type QueryResult<T> = std::result::Result<T, QueryError>;

// QueryError is Send + Sync because all variants either hold no data, a String,
// or an anyhow::Error (which requires Send + Sync on its inner error).  The
// compiler enforces this automatically via the trait impls on those types.
