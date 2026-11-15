//! Pure, versioned SQLite text projection. Never performs I/O or grants access.
//! Canonical JSON and HBS state remain owned by their original repositories.
use research_note_query::{note_display_title as display_title, note_fold};
use rusqlite::{Connection, functions::FunctionFlags};
use serde::Serialize;
use serde_json::Value;

use crate::notes::{
    document_validation::{known_block, validate_document},
    model::{NoteDocument, NoteDocumentBlock},
};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Projection {
    text: String,
    folded_text: String,
    first_line: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    error_code: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cause: Option<String>,
}

fn string(value: &Value, key: &str) -> String {
    value[key].as_str().unwrap_or_default().to_owned()
}

fn text_parts(value: &Value) -> String {
    value
        .as_array()
        .into_iter()
        .flatten()
        .map(|item| string(item, "text"))
        .collect()
}

fn inline_text(value: &Value) -> String {
    value["items"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|item| match item["type"].as_str() {
            Some("text") => string(item, "text"),
            Some("link") => text_parts(&item["content"]),
            Some("inlineFormula") => string(item, "source"),
            Some("mention") if item.get("content").is_some() => text_parts(&item["content"]),
            Some("mention") => string(item, "labelSnapshot"),
            _ => String::new(),
        })
        .collect()
}

fn block_text(value: &Value) -> String {
    match value["kind"].as_str() {
        Some("inline") => inline_text(value),
        Some("formula") => string(value, "source"),
        Some("asset") => [string(value, "displayName"), string(value, "caption")]
            .into_iter()
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join(" "),
        Some("reference") => [
            string(value, "labelSnapshot"),
            string(value, "quoteSnapshot"),
        ]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" "),
        Some("table") => value["rows"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|row| {
                row.as_array()
                    .into_iter()
                    .flatten()
                    .map(inline_text)
                    .collect::<Vec<_>>()
                    .join("\t")
            })
            .filter(|row| !row.is_empty())
            .collect::<Vec<_>>()
            .join("\n"),
        Some("unsupported") => format!("[不支持的块：{}]", string(value, "originalType")),
        _ => String::new(),
    }
}

/// Caller validates the canonical tree once. Reuses the block text semantics of
/// the persisted catalog; never reads history, recovery candidates or files.
pub(crate) fn document_text(document: &NoteDocument) -> String {
    fn visit(blocks: &[NoteDocumentBlock], parts: &mut Vec<String>) {
        for block in blocks {
            let text = block_text(&block.content);
            if !text.is_empty() {
                parts.push(text);
            }
            visit(&block.children, parts);
        }
    }
    let mut parts = Vec::new();
    visit(&document.blocks, &mut parts);
    parts.join("\n")
}

fn project(block_type: String, props: &str, content: &str) -> Projection {
    let result = (|| {
        if !known_block(&block_type) {
            // Match the frontend decoder's future-block placeholder, never index
            // unknown raw payloads. No content or unknown type appears in errors.
            if block_type.len() > 256 {
                return Err("原始块类型过长".to_owned());
            }
            return Ok(format!("[不支持的块：{block_type}]"));
        }
        let props = serde_json::from_str(props).map_err(|_| "笔记块属性 JSON 无效".to_owned())?;
        let content =
            serde_json::from_str(content).map_err(|_| "笔记块内容 JSON 无效".to_owned())?;
        let block = NoteDocumentBlock {
            id: "projection".to_owned(),
            block_type,
            props,
            content,
            children: vec![],
        };
        let document = NoteDocument {
            title: None,
            schema_version: 1,
            page_id: "projection".to_owned(),
            blocks: vec![block],
        };
        validate_document(&document)?;
        Ok(block_text(&document.blocks[0].content))
    })();
    match result {
        Ok(text) => {
            let first_line = display_title(&text, "", "");
            Projection {
                folded_text: note_fold(&text),
                text,
                first_line,
                error_code: None,
                cause: None,
            }
        }
        Err(cause) => Projection {
            text: String::new(),
            folded_text: String::new(),
            first_line: String::new(),
            error_code: Some("note_catalog_invalid_text"),
            cause: Some(cause),
        },
    }
}

/// Register before migration or any note write on each connection. The schema
/// calls v1 explicitly: future text semantics require a new projection version.
pub(crate) fn register_projection_functions(connection: &Connection) -> rusqlite::Result<()> {
    super::order_keys::register(connection)?;
    connection.create_scalar_function(
        "ra_note_display_title_v1",
        3,
        FunctionFlags::SQLITE_UTF8
            | FunctionFlags::SQLITE_DETERMINISTIC
            | FunctionFlags::SQLITE_INNOCUOUS,
        |context| {
            Ok(display_title(
                &context.get::<String>(0)?,
                &context.get::<String>(1)?,
                &context.get::<String>(2)?,
            ))
        },
    )?;
    connection.create_scalar_function(
        "ra_note_fold_v1",
        1,
        FunctionFlags::SQLITE_UTF8
            | FunctionFlags::SQLITE_DETERMINISTIC
            | FunctionFlags::SQLITE_INNOCUOUS,
        |context| Ok(note_fold(&context.get::<String>(0)?)),
    )?;
    connection.create_scalar_function(
        "ra_note_block_projection_v1",
        3,
        FunctionFlags::SQLITE_UTF8
            | FunctionFlags::SQLITE_DETERMINISTIC
            | FunctionFlags::SQLITE_INNOCUOUS,
        |context| {
            let block_type = context.get::<String>(0)?;
            let props = context.get::<String>(1)?;
            let content = context.get::<String>(2)?;
            serde_json::to_string(&project(block_type, &props, &content))
                .map_err(|error| rusqlite::Error::UserFunctionError(Box::new(error)))
        },
    )
}
