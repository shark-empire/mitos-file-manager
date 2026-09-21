use crate::error::FileManagerError;
use std::path::Path;

pub fn delete(path: &Path) -> Result<(), FileManagerError> {
    // Trashing `/usr` or `~` is just a slower way of deleting them.
    crate::filesystem::protection::ensure_modifiable(&[path.to_path_buf()])?;

    trash::delete(path).map_err(|err| FileManagerError::Trash(format!("{err:?}")))
}
