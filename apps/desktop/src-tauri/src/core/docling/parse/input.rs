//! The original settings/path/ownership/hash prefix, before any service wait.
use super::{AppStore, DoclingError, repository};
use crate::docling::model::DoclingSettings;
use std::path::PathBuf;

type Input = (DoclingSettings, PathBuf, String, String);

pub(super) async fn prepare(
    store: &AppStore,
    item_id: &str,
    attachment_id: &str,
) -> Result<Input, DoclingError> {
    if let Some(binding) = store.native_note_trace()
        && store.paper_reading_source().is_none()
    {
        // New package-file admission is blocking. The complete prefix runs in
        // one worker, whose database guard is dropped before this await ends.
        let package_key = binding.package_key.clone();
        let store = store.clone();
        let item_id = item_id.to_owned();
        let attachment_id = attachment_id.to_owned();
        return tokio::task::spawn_blocking(move || capture(&store, &item_id, &attachment_id))
            .await
            .map_err(|_| {
                DoclingError::Attachment(format!(
                    "plugin_attachment_path_worker_failed: 附件定位任务未完成。 [packageKey={package_key}]"
                ))
            })?;
    }
    capture(store, item_id, attachment_id)
}

fn capture(store: &AppStore, item_id: &str, attachment_id: &str) -> Result<Input, DoclingError> {
    let connection = store.connection()?;
    let settings = repository::get_settings(&connection)?;
    let file_path = if let Some(source) = store.paper_reading_source() {
        (source.attachment_path)(&connection, store.content_directory(), attachment_id)
            .map_err(DoclingError::Attachment)?
    } else if let Some(binding) = store.native_note_trace() {
        binding
            .service
            .docling_attachment_path(
                &connection,
                &binding.package_key,
                store.content_directory(),
                attachment_id,
            )
            .map_err(|error| {
                let mut message = format!("{error} [packageKey={}]", binding.package_key);
                if let Some(data) = error.data_error.as_deref() {
                    message.push_str(&format!(
                        " [resourceId={} resourceVersion={} dataCode={} phase={} safeCause={}]",
                        data.resource_id,
                        data.resource_version,
                        data.code,
                        data.phase,
                        data.safe_cause,
                    ));
                    if let Some(code) = &data.sqlite_code {
                        message.push_str(&format!(" [sqliteCode={code}]"));
                    }
                    if let Some(code) = data.sqlite_extended_code {
                        message.push_str(&format!(" [sqliteExtendedCode={code}]"));
                    }
                }
                DoclingError::Attachment(message)
            })?
    } else {
        return Err(DoclingError::Attachment(
            "CAPABILITY_UNAVAILABLE: literature_library.attachment_path".into(),
        ));
    };
    let (attachment_item_id, display_name): (String, String) = connection.query_row(
        "select item_id, display_name from attachments where id = ?1",
        [attachment_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    if attachment_item_id != item_id {
        return Err(DoclingError::Attachment("附件不属于当前文献".to_owned()));
    }
    let content_sha256 = repository::content_sha256_for_attachment(&connection, attachment_id)?;
    Ok((settings, file_path, display_name, content_sha256))
}

#[cfg(all(test, unix))]
#[path = "input/tests.rs"]
mod tests;
