use std::fmt;
use std::io;

#[derive(Debug)]
pub enum FileManagerError {
    Io(io::Error),
    Trash(String),
    InvalidName,
    NotADirectory,
}

impl fmt::Display for FileManagerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(err) => write!(f, "I/O error: {err}"),
            Self::Trash(message) => write!(f, "Trash error: {message}"),
            Self::InvalidName => write!(f, "Invalid name"),
            Self::NotADirectory => write!(f, "Not a directory"),
        }
    }
}

impl std::error::Error for FileManagerError {}

impl From<io::Error> for FileManagerError {
    fn from(err: io::Error) -> Self {
        Self::Io(err)
    }
}
