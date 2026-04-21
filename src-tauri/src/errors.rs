use thiserror::Error;

#[derive(Error, Debug)]
pub enum AppError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("http: {0}")]
    Http(#[from] reqwest::Error),
    #[error("not authenticated: {0}")]
    NotAuthenticated(String),
    #[error("token expired")]
    Expired,
    #[error("api error {status}: {message}")]
    Api { status: u16, message: String },
    #[error("other: {0}")]
    Other(String),
}

impl serde::Serialize for AppError {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}

pub type AppResult<T> = Result<T, AppError>;
