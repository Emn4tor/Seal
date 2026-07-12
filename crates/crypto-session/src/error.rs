#[derive(Debug, thiserror::Error)]
pub enum CryptoError {
    #[error("olm session creation error: {0}")]
    SessionCreation(#[from] vodozemac::olm::SessionCreationError),
    #[error("olm decryption error: {0}")]
    OlmDecryption(#[from] vodozemac::olm::DecryptionError),
    #[error("olm encryption error: {0}")]
    OlmEncryption(#[from] vodozemac::olm::EncryptionError),
    #[error("megolm decryption error: {0}")]
    MegolmDecryption(#[from] vodozemac::megolm::DecryptionError),
    #[error("invalid key material: {0}")]
    InvalidKey(String),
    #[error("message decode error: {0}")]
    Decode(String),
    #[error("no active session with peer")]
    NoSession,
    #[error("unknown group session")]
    UnknownGroupSession,
    #[error("expected a pre-key message to start an inbound session")]
    NotAPreKeyMessage,
}
