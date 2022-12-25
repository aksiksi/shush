/// Common error type.
#[derive(thiserror::Error, Debug)]
pub enum Error {
    /// Wraps [ffmpeg_next::Error].
    #[error("FFmpeg error: {0}")]
    FFmpegError(#[from] ffmpeg_next::Error),
    /// Wraps [whisper_rs::WhisperError].
    #[error("Whisper error: {0:?}")]
    WhisperError(whisper_rs::WhisperError),
    /// Wraps [std::io::Error].
    #[error("IO error: {0}")]
    IOError(#[from] std::io::Error),
}

/// Common result type.
pub type Result<T> = std::result::Result<T, Error>;

impl From<whisper_rs::WhisperError> for Error {
    fn from(e: whisper_rs::WhisperError) -> Self {
        Self::WhisperError(e)
    }
}
