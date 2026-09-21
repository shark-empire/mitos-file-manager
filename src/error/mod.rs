use std::fmt;
use std::io;

#[derive(Debug)]
pub enum FileManagerError {
    Io(io::Error),
    Trash(String),
    /// A file or folder name that can't be used: empty, `.` / `..`, or one
    /// containing a path separator (see `operations::validate_name`).
    InvalidName,
    /// A folder was required -- somewhere to paste into, a parent to create
    /// something in, a location to browse -- but the path is something else.
    NotADirectory,
}

impl fmt::Display for FileManagerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            // These three are what people actually hit, so they get plain
            // wording instead of the raw "os error 17" text.
            Self::Io(err) => match err.kind() {
                io::ErrorKind::AlreadyExists => {
                    write!(f, "An item with that name already exists")
                }
                io::ErrorKind::PermissionDenied => write!(f, "Permission denied"),
                io::ErrorKind::NotFound => write!(f, "No such file or folder"),
                _ => write!(f, "I/O error: {err}"),
            },
            Self::Trash(message) => write!(f, "Trash error: {message}"),
            Self::InvalidName => write!(
                f,
                "Invalid name -- it can't be empty, \".\", \"..\", or contain \"/\""
            ),
            Self::NotADirectory => write!(f, "Not a folder"),
        }
    }
}

impl std::error::Error for FileManagerError {}

impl From<io::Error> for FileManagerError {
    fn from(err: io::Error) -> Self {
        Self::Io(err)
    }
}
