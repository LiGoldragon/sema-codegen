use std::fmt;

#[derive(Debug)]
pub enum Error {
    Schema(String),
    Query(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Schema(detail) => write!(f, "schema error: {detail}"),
            Error::Query(detail) => write!(f, "query error: {detail}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<String> for Error {
    fn from(s: String) -> Self {
        Error::Query(s)
    }
}

impl From<criome_cozo::Error> for Error {
    fn from(e: criome_cozo::Error) -> Self {
        Error::Query(e.to_string())
    }
}
