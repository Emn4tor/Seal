#[derive(Debug, thiserror::Error)]
pub enum NetError {
    #[error("http request error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("directory server rejected request: {0}")]
    DirectoryRequestFailed(String),
}
