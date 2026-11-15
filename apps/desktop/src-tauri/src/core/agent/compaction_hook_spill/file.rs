//! Host-authorized recovery locators; Unix publication uses opened directory
//! handles, while other platforms retain the existing tempfile adapter.
use super::*;
use std::path::PathBuf;

#[cfg(unix)]
#[path = "file/unix.rs"]
mod unix;

/// Root must already exist, be absolute/canonical, and be readable by authorized
/// agent tools. This adapter grants no new tool permissions and has no default.
pub struct FileHookSpillStore {
    root: PathBuf,
}
impl FileHookSpillStore {
    pub fn new(root: PathBuf) -> Result<Self, HookContextError> {
        if !root.is_absolute() {
            return Err(HookContextError);
        }
        Ok(Self { root })
    }
}
impl HookSpillStore for FileHookSpillStore {
    fn save(&self, request: CompactionHookRequest, text: String) -> HookSpillFuture<'_> {
        let root = self.root.clone();
        Box::pin(async move {
            tokio::task::spawn_blocking(move || {
                if request.conversation_id.is_nil()
                    || request.operation_id.is_nil()
                    || text.len() > 32 * 1024 * 1024
                    || root.canonicalize().map_err(|_| HookContextError)? != root
                    || !root.is_dir()
                {
                    return Err(HookContextError);
                }
                let target = root.join(format!(
                    "hook-output-{}-{}-{}.txt",
                    request.conversation_id,
                    request.operation_id,
                    uuid::Uuid::new_v4()
                ));
                let locator = target.to_str().ok_or(HookContextError)?.to_owned();
                if locator.contains(['\n', '\r', '\0']) {
                    return Err(HookContextError);
                }
                #[cfg(unix)]
                unix::save(&root, &target, text.as_bytes())?;
                #[cfg(not(unix))]
                save_portable(&root, &target, text.as_bytes())?;
                Ok(locator)
            })
            .await
            .map_err(|_| HookContextError)?
        })
    }
}

// Unix uses opened directory handles below. Preserve the existing portable
// adapter on platforms without that native filesystem boundary.
#[cfg(not(unix))]
fn save_portable(
    root: &std::path::Path,
    target: &std::path::Path,
    text: &[u8],
) -> Result<(), HookContextError> {
    use std::io::Write;
    let mut file = tempfile::NamedTempFile::new_in(root).map_err(|_| HookContextError)?;
    file.write_all(text).map_err(|_| HookContextError)?;
    file.as_file().sync_all().map_err(|_| HookContextError)?;
    file.persist_noclobber(target)
        .map_err(|_| HookContextError)?;
    Ok(())
}
