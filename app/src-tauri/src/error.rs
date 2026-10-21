use serde::{Serialize, Serializer};

#[derive(Debug)]
pub enum Error {
    Db(rusqlite::Error),
    Invalid(String),
    NotFound,
}

pub type Result<T> = std::result::Result<T, Error>;

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Db(e) => write!(f, "database error: {e}"),
            Error::Invalid(msg) => f.write_str(msg),
            Error::NotFound => f.write_str("item not found"),
        }
    }
}

impl std::error::Error for Error {}

impl From<rusqlite::Error> for Error {
    fn from(e: rusqlite::Error) -> Self {
        Error::Db(e)
    }
}

/// Commands reject with the error message as a plain string.
impl Serialize for Error {
    fn serialize<S: Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}
