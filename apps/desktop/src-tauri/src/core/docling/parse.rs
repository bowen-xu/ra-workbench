//! Native conversion boundary; shared preparation owns cache and concurrent requests.
use super::{
    error::DoclingError,
    manager::DoclingServiceManager,
    model::DoclingParseRunDetail,
    repository::{self, ParsedDoclingResponse},
};
use crate::storage::AppStore;
use serde_json::Value;

#[path = "parse/input.rs"]
mod input;

pub(super) async fn convert(
    store: &AppStore,
    manager: &DoclingServiceManager,
    item_id: String,
    attachment_id: String,
    control: std::sync::Arc<super::task_control::ParseControl>,
) -> Result<DoclingParseRunDetail, DoclingError> {
    let (settings, file_path, display_name, content_sha256) =
        input::prepare(store, &item_id, &attachment_id).await?;

    control.stage("connecting");
    let connect = manager.require_connected(&settings);
    tokio::pin!(connect);
    let mut tick = tokio::time::interval(std::time::Duration::from_millis(300));
    let service = loop {
        tokio::select! {
            result = &mut connect => break result?,
            _ = control.cancelled() => return Err(DoclingError::Cancelled),
            _ = tick.tick() => { if manager.status().state == "starting" { control.stage("starting_service"); } }
        }
    };
    let raw_response = manager
        .convert_pdf(
            &service,
            &file_path,
            &display_name,
            settings.request_timeout_ms,
            control.clone(),
        )
        .await?;
    let docling_status = raw_response
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_owned();
    let (parse_status, json_content) = parsed_document(&raw_response);
    let error_message = raw_response
        .get("errors")
        .and_then(Value::as_array)
        .filter(|errors| !errors.is_empty())
        .map(|errors| {
            errors
                .iter()
                .map(Value::to_string)
                .collect::<Vec<_>>()
                .join("\n")
        });
    let processing_time_seconds = raw_response.get("processing_time").and_then(Value::as_f64);

    if control.is_cancelled() {
        return Err(DoclingError::Cancelled);
    }
    control.stage("saving");
    let connection = store.connection()?;
    repository::save_parse_response(
        &connection,
        ParsedDoclingResponse {
            item_id,
            attachment_id,
            content_sha256,
            service_base_url: service.base_url,
            docling_status,
            parse_status: parse_status.to_owned(),
            json_content,
            raw_response,
            error_message,
            processing_time_seconds,
        },
    )
}

fn parsed_document(response: &Value) -> (&str, Option<Value>) {
    let document = response
        .get("document")
        .and_then(|value| value.get("json_content"))
        .filter(|value| value.is_object())
        .cloned();
    let status = match response.get("status").and_then(Value::as_str) {
        Some("success") if document.is_some() => "success",
        Some("partial_success") if document.is_some() => "partial_success",
        _ => "failure",
    };
    (status, document)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn successful_status_requires_a_structured_document() {
        for document in [Value::Null, json!("text"), json!([])] {
            assert_eq!(
                parsed_document(&json!({"status":"success","document":{"json_content":document}}))
                    .0,
                "failure"
            );
        }
        assert_eq!(parsed_document(&json!({"status":"success"})).0, "failure");
        for status in ["success", "partial_success"] {
            assert_eq!(
                parsed_document(&json!({"status":status,"document":{"json_content":{"texts":[]}}}))
                    .0,
                status
            );
        }
    }
}
