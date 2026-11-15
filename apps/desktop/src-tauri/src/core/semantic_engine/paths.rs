use std::path::{Component, Path, PathBuf};

use super::VectorError;

pub(super) fn controlled_database_path(
    index_root: &Path,
    relative_path: &str,
) -> Result<PathBuf, VectorError> {
    let relative = Path::new(relative_path);
    if relative.is_absolute()
        || relative
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(VectorError::InvalidInput("LanceDB generation 路径无效"));
    }
    Ok(index_root.join(relative))
}
