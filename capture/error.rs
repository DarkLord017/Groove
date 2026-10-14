use std::fmt;

#[derive(Debug)]
pub enum CaptureError {
    Io(std::io::Error),
    Json(serde_json::Error),
}

impl fmt::Display for CaptureError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CaptureError::Io(err) => write!(f, "io error: {err}"),
            CaptureError::Json(err) => write!(f, "json error: {err}"),
        }
    }
}

impl std::error::Error for CaptureError {}

impl From<std::io::Error> for CaptureError {
    fn from(err: std::io::Error) -> Self {
        CaptureError::Io(err)
    }
}

impl From<serde_json::Error> for CaptureError {
    fn from(err: serde_json::Error) -> Self {
        CaptureError::Json(err)
    }
}
