use crate::error::FileManagerError;
use std::path::Path;

pub fn delete(path: &Path) -> Result<(), FileManagerError> {
    trash::delete(path).map_err(|err| FileManagerError::Trash(format!("{err:?}")))
}
