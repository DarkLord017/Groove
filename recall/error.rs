use std::fmt;

#[derive(Debug)]
pub enum RecallError {
    Io(std::io::Error),
    Json(serde_json::Error),
}

impl fmt::Display for RecallError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RecallError::Io(err) => write!(f, "io error: {err}"),
            RecallError::Json(err) => write!(f, "json error: {err}"),
        }
    }
}

impl std::error::Error for RecallError {}

impl From<std::io::Error> for RecallError {
    fn from(err: std::io::Error) -> Self {
        RecallError::Io(err)
    }
}

impl From<serde_json::Error> for RecallError {
    fn from(err: serde_json::Error) -> Self {
        RecallError::Json(err)
    }
}
