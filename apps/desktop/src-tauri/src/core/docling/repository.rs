use rusqlite::{Connection, OptionalExtension, params};
use serde_json::Value;
use uuid::Uuid;

use super::{
    error::DoclingError,
    model::{DoclingParseRunDetail, DoclingParseRunSummary, DoclingSettings, DoclingSettingsDraft},
};

pub(crate) struct ParsedDoclingResponse {
    pub(crate) item_id: String,
    pub(crate) attachment_id: String,
    pub(crate) content_sha256: String,
    pub(crate) service_base_url: String,
    pub(crate) docling_status: String,
    pub(crate) parse_status: String,
    pub(crate) json_content: Option<Value>,
    pub(crate) raw_response: Value,
    pub(crate) error_message: Option<String>,
    pub(crate) processing_time_seconds: Option<f64>,
}

pub(crate) fn get_settings(connection: &Connection) -> Result<DoclingSettings, DoclingError> {
    connection
        .query_row(
            "select enabled, model_manifest_url, default_port, fallback_start_port, fallback_end_port,
                    idle_shutdown_ms, request_timeout_ms, updated_at
             from docling_settings where id = 1",
            [],
            |row| {
                Ok(DoclingSettings {
                    enabled: row.get::<_, bool>(0)?,
                    model_manifest_url: row.get(1)?,
                    default_port: row.get::<_, u16>(2)?,
                    fallback_start_port: row.get::<_, u16>(3)?,
                    fallback_end_port: row.get::<_, u16>(4)?,
                    idle_shutdown_ms: row.get::<_, u64>(5)?,
                    request_timeout_ms: row.get::<_, u64>(6)?,
                    updated_at: row.get(7)?,
                })
            },
        )
        .map_err(Into::into)
}

pub(crate) fn save_settings(
    connection: &Connection,
    draft: &DoclingSettingsDraft,
) -> Result<DoclingSettings, DoclingError> {
    validate_settings(draft)?;
    connection.execute(
        "insert into docling_settings (
             id, enabled, model_manifest_url, default_port, fallback_start_port,
             fallback_end_port, idle_shutdown_ms, request_timeout_ms, updated_at
         )
         values (1, ?1, ?2, ?3, ?4, ?5, ?6, ?7, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         on conflict(id) do update set
             enabled = excluded.enabled,
             model_manifest_url = excluded.model_manifest_url,
             default_port = excluded.default_port,
             fallback_start_port = excluded.fallback_start_port,
             fallback_end_port = excluded.fallback_end_port,
             idle_shutdown_ms = excluded.idle_shutdown_ms,
             request_timeout_ms = excluded.request_timeout_ms,
             updated_at = excluded.updated_at",
        params![
            draft.enabled,
            draft.model_manifest_url.trim(),
            draft.default_port,
            draft.fallback_start_port,
            draft.fallback_end_port,
            draft.idle_shutdown_ms,
            draft.request_timeout_ms,
        ],
    )?;
    get_settings(connection)
}

fn validate_settings(draft: &DoclingSettingsDraft) -> Result<(), DoclingError> {
    if draft.model_manifest_url.trim().is_empty() {
        return Err(DoclingError::InvalidSettings(
            "模型更新源不能为空".to_owned(),
        ));
    }
    if draft.model_manifest_url.len() > 2048 {
        return Err(DoclingError::InvalidSettings("模型更新源过长".to_owned()));
    }
    if draft.fallback_start_port > draft.fallback_end_port {
        return Err(DoclingError::InvalidSettings(
            "fallback 起始端口不能大于结束端口".to_owned(),
        ));
    }
    super::models::validated_url(&draft.model_manifest_url)?;
    Ok(())
}

pub(crate) fn content_sha256_for_attachment(
    connection: &Connection,
    attachment_id: &str,
) -> Result<String, DoclingError> {
    connection
        .query_row(
            "select content_sha256 from attachments where id = ?1",
            [attachment_id],
            |row| row.get(0),
        )
        .optional()?
        .ok_or(DoclingError::InvalidResponse(
            "找不到附件的内容哈希".to_owned(),
        ))
}

pub(crate) fn save_parse_response(
    connection: &Connection,
    response: ParsedDoclingResponse,
) -> Result<DoclingParseRunDetail, DoclingError> {
    let id = Uuid::new_v4().to_string();
    let raw_response = serde_json::to_string(&response.raw_response)
        .map_err(|error| DoclingError::InvalidResponse(error.to_string()))?;
    let json_content = response
        .json_content
        .as_ref()
        .map(serde_json::to_string)
        .transpose()
        .map_err(|error| DoclingError::InvalidResponse(error.to_string()))?;
    connection.execute(
        "insert into docling_parse_runs (
             id, item_id, attachment_id, content_sha256, service_base_url,
             docling_status, parse_status, json_content, raw_response, error_message,
             processing_time_seconds, created_at, finished_at
         )
         values (
             ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11,
             strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
             strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         )",
        params![
            id,
            response.item_id,
            response.attachment_id,
            response.content_sha256,
            response.service_base_url,
            response.docling_status,
            response.parse_status,
            json_content,
            raw_response,
            response.error_message,
            response.processing_time_seconds,
        ],
    )?;
    get_parse_run(connection, &id)
}

pub(crate) fn list_parse_runs(
    connection: &Connection,
    item_id: &str,
) -> Result<Vec<DoclingParseRunSummary>, DoclingError> {
    let mut statement = connection.prepare(
        "select id, item_id, attachment_id, content_sha256, service_base_url,
                docling_status, parse_status, error_message, processing_time_seconds,
                created_at, finished_at
         from docling_parse_runs
         where item_id = ?1
         order by created_at desc
         limit 100",
    )?;
    let rows = statement.query_map([item_id], read_summary)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

pub(crate) fn get_parse_run(
    connection: &Connection,
    run_id: &str,
) -> Result<DoclingParseRunDetail, DoclingError> {
    connection
        .query_row(
            "select id, item_id, attachment_id, content_sha256, service_base_url,
                    docling_status, parse_status, error_message, processing_time_seconds,
                    created_at, finished_at, json_content, raw_response
             from docling_parse_runs
             where id = ?1",
            [run_id],
            |row| {
                let summary = DoclingParseRunSummary {
                    id: row.get(0)?,
                    item_id: row.get(1)?,
                    attachment_id: row.get(2)?,
                    content_sha256: row.get(3)?,
                    service_base_url: row.get(4)?,
                    docling_status: row.get(5)?,
                    parse_status: row.get(6)?,
                    error_message: row.get(7)?,
                    processing_time_seconds: row.get(8)?,
                    created_at: row.get(9)?,
                    finished_at: row.get(10)?,
                };
                let json_content: Option<String> = row.get(11)?;
                let raw_response: String = row.get(12)?;
                Ok((summary, json_content, raw_response))
            },
        )
        .optional()?
        .map(
            |(summary, json_content, raw_response)| -> Result<DoclingParseRunDetail, DoclingError> {
                let json_content = json_content
                    .map(|value| serde_json::from_str(&value))
                    .transpose()
                    .map_err(|error| DoclingError::InvalidResponse(error.to_string()))?;
                let raw_response = serde_json::from_str(&raw_response)
                    .map_err(|error| DoclingError::InvalidResponse(error.to_string()))?;
                Ok(DoclingParseRunDetail {
                    summary,
                    json_content,
                    raw_response,
                })
            },
        )
        .transpose()?
        .ok_or(DoclingError::ParseRunNotFound)
}

fn read_summary(row: &rusqlite::Row<'_>) -> rusqlite::Result<DoclingParseRunSummary> {
    Ok(DoclingParseRunSummary {
        id: row.get(0)?,
        item_id: row.get(1)?,
        attachment_id: row.get(2)?,
        content_sha256: row.get(3)?,
        service_base_url: row.get(4)?,
        docling_status: row.get(5)?,
        parse_status: row.get(6)?,
        error_message: row.get(7)?,
        processing_time_seconds: row.get(8)?,
        created_at: row.get(9)?,
        finished_at: row.get(10)?,
    })
}
