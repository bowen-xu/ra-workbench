use super::*;
use crate::content::StoredObjectReadError;
use std::fs;

pub(super) fn remove(
    store: &AppStore,
    resources: &AttachmentResources,
    item: &str,
    attachment: &str,
) -> Result<AttachmentRemovalResult> {
    let mut progress = AttachmentProgress::new("remove-owner");
    progress.attachment_id = Some(attachment.into());
    let _content = store.content_write().map_err(|e| progress.owner(e))?;
    let mut db = store.connection().map_err(|e| progress.owner(e))?;
    progress.phase = "remove-header";
    let rows = resources
        .read(&db, "remove-header", vec![text(item), text(attachment)])
        .map_err(|e| progress.data(e))?;
    let hash = match rows.rows.first().map(Vec::as_slice) {
        None => return Err(progress.missing()),
        Some([DataValue::Text(hash)]) => hash,
        _ => return Err(progress.data(resources.shape_error("remove-header"))),
    };
    let relative = relative_path(&db, resources, attachment, &progress)?;
    progress.phase = "remove-path";
    let source = match StoredObjectRead::resolve(store.content_directory(), &relative) {
        Ok(read) => {
            let path = read.into_legacy_path();
            // Core's existing exact physical-file gate is stricter for unlink
            // than ordinary read/reveal, which permit an internal symlink.
            let raw: String = db
                .query_row(
                    "select relative_path from content_objects where sha256=?1",
                    [hash],
                    |r| r.get(0),
                )
                .map_err(|e| progress.sql_error(&e))?;
            if store
                .content_directory()
                .canonicalize()
                .map_err(|e| progress.io(e))?
                .join(raw)
                != path
            {
                return Err(progress.fail("storage_error", "附件存储路径不安全"));
            }
            Some(path)
        }
        Err(StoredObjectReadError::MissingFile) => None,
        Err(StoredObjectReadError::Io(cause)) if cause.kind() == std::io::ErrorKind::NotFound => {
            None
        }
        Err(cause) => return Err(progress.stored(cause)),
    };
    let retained = store
        .is_content_session_retained(hash)
        .map_err(|e| progress.owner(e))?;
    let referenced = transaction::run(&mut db, |tx, progress| {
        progress.phase = "remove-delete";
        progress.attachment_id = Some(attachment.into());
        if let Err(error) = resources.write(tx, "remove-delete", vec![text(attachment), text(item)])
        {
            if foreign_key(&error) {
                return Err(progress.fail(
                    "attachment_in_use",
                    "附件仍被已保存的研究上下文或证据引用，无法删除；原记录和文件均保留",
                ));
            }
            return Err(progress.data(error));
        }
        progress.phase = "remove-references";
        let referenced = tx
            .query_row(
                "select exists(select 1 from content_object_references where content_sha256=?1)",
                [hash],
                |r| r.get::<_, bool>(0),
            )
            .map_err(|e| progress.sql_error(&e))?;
        progress.phase = "remove-fts";
        resources
            .write(tx, "remove-fts", vec![text(item)])
            .map_err(|e| progress.data(e))?;
        Ok(referenced)
    })?;
    if referenced || retained {
        return Ok(AttachmentRemovalResult {
            file_status: if source.is_none() {
                "missing"
            } else {
                "retained"
            },
            warning: None,
        });
    }
    let status = match source {
        Some(path) => match fs::remove_file(path) {
            Ok(()) => "deleted",
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => "missing",
            Err(error) => {
                return Ok(AttachmentRemovalResult {
                    file_status: "cleanup_failed",
                    warning: Some(format!(
                        "ATTACHMENT_FILE_CLEANUP_FAILED：附件记录已删除，文件清理失败，保留待重试：{error}"
                    )),
                });
            }
        },
        None => "missing",
    };
    let warning = db.execute("delete from content_objects where sha256=?1 and not exists(select 1 from content_object_references where content_sha256=?1)", [hash])
        .err().map(|error| format!("ATTACHMENT_STORAGE_CLEANUP_FAILED：附件记录已删除，存储记录待清理：{error}"));
    Ok(AttachmentRemovalResult {
        file_status: status,
        warning,
    })
}

fn foreign_key(error: &crate::data_interface::plugin_resources::PluginDataError) -> bool {
    error.sqlite_extended_code == Some(rusqlite::ffi::SQLITE_CONSTRAINT_FOREIGNKEY)
        || (error.sqlite_extended_code == Some(rusqlite::ffi::SQLITE_CONSTRAINT_TRIGGER)
            && error
                .original_sql_failure
                .as_ref()
                .is_some_and(|e| e.message() == "FOREIGN KEY constraint failed"))
}
