use std::collections::HashSet;

use serde_json::{Map, Value};

use super::model::{NoteDocument, NoteDocumentBlock};

const MAX_DOCUMENT_BLOCKS: usize = 20_000;
const MAX_DOCUMENT_DEPTH: usize = 64;
const MAX_DOCUMENT_TEXT_BYTES: usize = 8 * 1024 * 1024;
const MAX_INLINE_ITEMS: usize = 100_000;
const MAX_TABLE_CELLS: usize = 10_000;

#[derive(Default)]
struct ValidationBudget {
    block_count: usize,
    inline_item_count: usize,
    table_cell_count: usize,
    text_bytes: usize,
    external_asset_ids: bool,
}

fn object<'a>(value: &'a Value, label: &str) -> Result<&'a Map<String, Value>, String> {
    value
        .as_object()
        .ok_or_else(|| format!("{label} 必须是对象"))
}

fn text<'a>(
    object: &'a Map<String, Value>,
    key: &str,
    label: &str,
    max_bytes: usize,
    budget: &mut ValidationBudget,
) -> Result<&'a str, String> {
    let value = object
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{label} 无效"))?;
    if value.len() > max_bytes {
        return Err(format!("{label} 过长"));
    }
    budget.text_bytes = budget.text_bytes.saturating_add(value.len());
    if budget.text_bytes > MAX_DOCUMENT_TEXT_BYTES {
        return Err("笔记文本总量超过安全限制".to_owned());
    }
    Ok(value)
}

fn validate_caption(value: &Value, label: &str) -> Result<(), String> {
    let value = value.as_str().ok_or_else(|| format!("{label} 无效"))?;
    // Match JavaScript string length and the shared editor's text limit.
    // The document budget continues to count the original UTF-8 bytes.
    if value.encode_utf16().count() > 4_000 {
        return Err(format!("{label} 过长"));
    }
    Ok(())
}

fn valid_target_kind(value: &str) -> bool {
    matches!(
        value,
        "note_page" | "reading_note" | "paper" | "claim" | "manuscript"
    )
}

fn valid_href(value: &str) -> bool {
    if value.starts_with('#') {
        return true;
    }
    value.split_once(':').is_some_and(|(scheme, _)| {
        matches!(
            scheme.to_ascii_lowercase().as_str(),
            "https" | "http" | "mailto" | "source"
        )
    })
}

fn validate_text_item(
    item: &Map<String, Value>,
    budget: &mut ValidationBudget,
) -> Result<(), String> {
    text(item, "text", "文本", 2_000_000, budget)?;
    let styles = item
        .get("styles")
        .and_then(Value::as_array)
        .ok_or_else(|| "文本样式无效".to_owned())?;
    if styles.iter().any(|style| {
        !style.as_str().is_some_and(|value| {
            matches!(value, "bold" | "italic" | "underline" | "strike" | "code")
        })
    }) {
        return Err("文本样式无效".to_owned());
    }
    for key in ["textColor", "backgroundColor"] {
        if let Some(color) = item.get(key)
            && !color.as_str().is_some_and(|value| {
                matches!(
                    value,
                    "default"
                        | "gray"
                        | "brown"
                        | "orange"
                        | "yellow"
                        | "green"
                        | "blue"
                        | "purple"
                        | "pink"
                        | "red"
                )
            })
        {
            return Err("文本颜色无效".to_owned());
        }
    }
    Ok(())
}

fn validate_inline_item(value: &Value, budget: &mut ValidationBudget) -> Result<(), String> {
    budget.inline_item_count += 1;
    if budget.inline_item_count > MAX_INLINE_ITEMS {
        return Err("笔记行内内容超过安全限制".to_owned());
    }
    let item = object(value, "行内内容")?;
    let item_type = item
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| "行内内容类型无效".to_owned())?;
    match item_type {
        "text" => validate_text_item(item, budget),
        "link" => {
            let href = text(item, "href", "链接", 4_096, budget)?;
            if !valid_href(href) {
                return Err("链接协议不安全或不受支持".to_owned());
            }
            let content = item
                .get("content")
                .and_then(Value::as_array)
                .ok_or_else(|| "链接文本无效".to_owned())?;
            for part in content {
                let part = object(part, "链接文本")?;
                if part.get("type").and_then(Value::as_str) != Some("text") {
                    return Err("链接只能包含文本".to_owned());
                }
                validate_text_item(part, budget)?;
            }
            Ok(())
        }
        "inlineFormula" => {
            text(item, "source", "行内公式", 32_768, budget)?;
            Ok(())
        }
        "mention" => {
            let kind = text(item, "targetKind", "提及目标类型", 32, budget)?;
            if !valid_target_kind(kind) {
                return Err("提及目标类型无效".to_owned());
            }
            text(item, "targetId", "提及目标", 128, budget)?;
            text(item, "labelSnapshot", "提及标签", 1_000, budget)?;
            if let Some(content) = item.get("content") {
                for part in content.as_array().ok_or("引用文本无效")? {
                    if part.get("type").and_then(Value::as_str) != Some("text") {
                        return Err("引用名称只能包含文本".to_owned());
                    }
                    validate_inline_item(part, budget)?;
                }
            }
            Ok(())
        }
        _ => Err("不支持的行内内容".to_owned()),
    }
}

fn validate_inline_container(value: &Value, budget: &mut ValidationBudget) -> Result<(), String> {
    let content = object(value, "行内块内容")?;
    if content.get("kind").and_then(Value::as_str) != Some("inline") {
        return Err("行内块内容无效".to_owned());
    }
    let items = content
        .get("items")
        .and_then(Value::as_array)
        .ok_or_else(|| "行内块内容无效".to_owned())?;
    for item in items {
        validate_inline_item(item, budget)?;
    }
    Ok(())
}

fn validate_props(value: &Value, budget: &mut ValidationBudget) -> Result<(), String> {
    for (key, value) in object(value, "块属性")? {
        if key.len() > 128 {
            return Err("块属性名称过长".to_owned());
        }
        match value {
            Value::String(text) => {
                budget.text_bytes = budget.text_bytes.saturating_add(text.len());
                if budget.text_bytes > MAX_DOCUMENT_TEXT_BYTES {
                    return Err("笔记文本总量超过安全限制".to_owned());
                }
            }
            Value::Number(_) | Value::Bool(_) => {}
            _ => return Err("块属性值无效".to_owned()),
        }
    }
    Ok(())
}

fn validate_content(
    block: &NoteDocumentBlock,
    budget: &mut ValidationBudget,
) -> Result<(), String> {
    let content = object(&block.content, "块内容")?;
    let kind = content
        .get("kind")
        .and_then(Value::as_str)
        .ok_or_else(|| "块内容类型无效".to_owned())?;
    let expected_kind = match block.block_type.as_str() {
        "paragraph" | "heading" | "bulletListItem" | "numberedListItem" | "checkListItem"
        | "toggleListItem" | "quote" | "codeBlock" => "inline",
        "divider" => "none",
        "table" => "table",
        "image" | "file" => "asset",
        "formulaBlock" => "formula",
        "childPage" | "readingNoteReference" | "researchObjectReference" => "reference",
        "unsupported" => "unsupported",
        _ => return Err(format!("不支持的笔记块类型：{}", block.block_type)),
    };
    if kind != expected_kind {
        return Err(format!("笔记块 {} 的内容类型无效", block.block_type));
    }
    match kind {
        "inline" => validate_inline_container(&block.content, budget),
        "none" => Ok(()),
        "formula" => {
            text(content, "source", "公式", 32_768, budget)?;
            Ok(())
        }
        "asset" => {
            let asset_id = text(content, "assetId", "资产 ID", 128, budget)?;
            if !budget.external_asset_ids
                && (asset_id.len() != 64
                    || !asset_id.bytes().all(|value| value.is_ascii_hexdigit()))
            {
                return Err("资产 ID 无效".to_owned());
            }
            text(content, "displayName", "资产名称", 1_000, budget)?;
            text(content, "mimeType", "资产 MIME", 255, budget)?;
            validate_caption(content.get("caption").ok_or("资产说明 无效")?, "资产说明")?;
            text(
                content,
                "caption",
                "资产说明",
                MAX_DOCUMENT_TEXT_BYTES,
                budget,
            )?;
            Ok(())
        }
        "reference" => {
            let target_kind = text(content, "targetKind", "引用目标类型", 32, budget)?;
            if !valid_target_kind(target_kind) {
                return Err("引用目标类型无效".to_owned());
            }
            if block.block_type == "childPage" && target_kind != "note_page" {
                return Err("子页面块只能指向笔记页面".to_owned());
            }
            text(content, "targetId", "引用目标", 128, budget)?;
            text(content, "labelSnapshot", "引用标签", 1_000, budget)?;
            text(content, "quoteSnapshot", "引用快照", 2_000_000, budget)?;
            Ok(())
        }
        "table" => {
            for key in ["headerRows", "headerCols"] {
                if let Some(count) = content.get(key)
                    && count.as_u64().is_none_or(|value| value > 10_000)
                {
                    return Err("表格表头数量无效".to_owned());
                }
            }
            if let Some(caption) = block.props.get("caption") {
                validate_caption(caption, "表格说明")?;
            }
            let rows = content
                .get("rows")
                .and_then(Value::as_array)
                .ok_or_else(|| "表格内容无效".to_owned())?;
            for row in rows {
                let cells = row.as_array().ok_or_else(|| "表格行无效".to_owned())?;
                budget.table_cell_count += cells.len();
                if budget.table_cell_count > MAX_TABLE_CELLS {
                    return Err("表格单元格数量超过安全限制".to_owned());
                }
                for cell in cells {
                    validate_inline_container(cell, budget)?;
                }
            }
            Ok(())
        }
        "unsupported" => {
            text(content, "originalType", "原始块类型", 256, budget)?;
            text(content, "rawJson", "原始块数据", 2_000_000, budget)?;
            Ok(())
        }
        _ => Err("不支持的块内容".to_owned()),
    }
}

pub(crate) fn validate_document(document: &NoteDocument) -> Result<(), String> {
    validate_with_budget(document, ValidationBudget::default())
}

/// The external asset owner validates identity; Core retains structure and budgets.
pub(super) fn validate_document_with_external_assets(
    document: &NoteDocument,
) -> Result<(), String> {
    validate_with_budget(
        document,
        ValidationBudget {
            external_asset_ids: true,
            ..ValidationBudget::default()
        },
    )
}

fn validate_with_budget(
    document: &NoteDocument,
    mut budget: ValidationBudget,
) -> Result<(), String> {
    if document
        .title
        .as_ref()
        .is_some_and(|title| title.encode_utf16().count() > 240 || title.contains(['\r', '\n']))
    {
        return Err("页面标题须为单行文本，且不超过 240 个 UTF-16 单位".to_owned());
    }
    if document.schema_version != 1
        || document.page_id.trim().is_empty()
        || document.page_id.len() > 128
    {
        return Err("不支持的笔记文档版本或页面 ID".to_owned());
    }
    fn visit(
        block: &NoteDocumentBlock,
        depth: usize,
        budget: &mut ValidationBudget,
        ids: &mut HashSet<String>,
    ) -> Result<(), String> {
        budget.block_count += 1;
        if depth > MAX_DOCUMENT_DEPTH || budget.block_count > MAX_DOCUMENT_BLOCKS {
            return Err("笔记结构超过安全限制".to_owned());
        }
        if block.id.trim().is_empty() || block.id.len() > 128 || !ids.insert(block.id.clone()) {
            return Err("笔记块 ID 为空、过长或重复".to_owned());
        }
        validate_props(&block.props, budget)?;
        validate_content(block, budget)?;
        for child in &block.children {
            visit(child, depth + 1, budget, ids)?;
        }
        Ok(())
    }
    let mut ids = HashSet::new();
    for block in &document.blocks {
        visit(block, 1, &mut budget, &mut ids)?;
    }
    Ok(())
}
pub(crate) fn known_block(kind: &str) -> bool {
    matches!(
        kind,
        "paragraph"
            | "heading"
            | "bulletListItem"
            | "numberedListItem"
            | "checkListItem"
            | "toggleListItem"
            | "quote"
            | "codeBlock"
            | "divider"
            | "table"
            | "image"
            | "file"
            | "formulaBlock"
            | "childPage"
            | "readingNoteReference"
            | "researchObjectReference"
            | "unsupported"
    )
}
/// Future blocks may be retained verbatim or moved; this does not admit invented
/// unknown data through the ordinary save command.
pub(crate) fn validate_preserving(
    document: &NoteDocument,
    before: &NoteDocument,
) -> Result<(), String> {
    fn find<'a>(blocks: &'a [NoteDocumentBlock], id: &str) -> Option<&'a NoteDocumentBlock> {
        blocks.iter().find_map(|block| {
            if block.id == id {
                Some(block)
            } else {
                find(&block.children, id)
            }
        })
    }
    fn visit(blocks: &mut [NoteDocumentBlock], before: &NoteDocument) -> Result<(), String> {
        for block in blocks {
            if !known_block(&block.block_type) {
                let prior = find(&before.blocks, &block.id).ok_or("invented future block")?;
                if serde_json::to_value(&*block).map_err(|error| error.to_string())?
                    != serde_json::to_value(prior).map_err(|error| error.to_string())?
                {
                    return Err("future blocks must be preserved verbatim".into());
                }
                let raw = serde_json::to_string(&*block).map_err(|error| error.to_string())?;
                block.content = serde_json::json!({"kind":"unsupported","originalType":block.block_type,"rawJson":raw});
                block.block_type = "unsupported".into();
                block.props = serde_json::json!({});
                block.children.clear();
            } else {
                visit(&mut block.children, before)?;
            }
        }
        Ok(())
    }
    let mut surrogate = document.clone();
    visit(&mut surrogate.blocks, before)?;
    validate_document(&surrogate)
}

#[cfg(test)]
#[path = "document_caption_tests.rs"]
mod caption_tests;

#[cfg(test)]
mod tests {
    use super::*;

    fn document(content: Value) -> NoteDocument {
        NoteDocument {
            title: None,
            schema_version: 1,
            page_id: "page".to_owned(),
            blocks: vec![NoteDocumentBlock {
                id: "block".to_owned(),
                block_type: "paragraph".to_owned(),
                props: serde_json::json!({}),
                content,
                children: vec![],
            }],
        }
    }

    #[test]
    fn validates_document_semantics_at_the_ipc_boundary() {
        validate_document(&document(serde_json::json!({"kind":"inline","items":[{"type":"text","text":"result","styles":["bold"]}]}))).expect("valid document");
        assert!(validate_document(&document(serde_json::json!({"kind":"inline","items":[{"type":"link","href":"javascript:alert(1)","content":[]}]}))).expect_err("unsafe link").contains("协议"));
        assert!(
            validate_document(&document(
                serde_json::json!({"kind":"asset","assetId":"bad"})
            ))
            .expect_err("mismatched content")
            .contains("内容类型")
        );
    }
    #[test]
    fn validates_table_header_counts_without_requiring_them_for_older_tables() {
        let mut doc =
            document(serde_json::json!({"kind":"table","rows":[[{"kind":"inline","items":[]}]]}));
        doc.blocks[0].block_type = "table".to_owned();
        validate_document(&doc).unwrap();
        for key in ["headerRows", "headerCols"] {
            for count in [serde_json::json!(0), serde_json::json!(1)] {
                doc.blocks[0].content[key] = count;
                validate_document(&doc).unwrap();
            }
            for count in [
                serde_json::json!(-1),
                serde_json::json!(1.5),
                serde_json::json!("1"),
                serde_json::json!(10_001),
            ] {
                doc.blocks[0].content[key] = count;
                assert!(validate_document(&doc).unwrap_err().contains("表头"));
            }
            doc.blocks[0].content.as_object_mut().unwrap().remove(key);
        }
    }
    #[test]
    fn validates_styled_reference_labels() {
        let content = serde_json::json!({"kind":"inline","items":[{"type":"mention","targetKind":"paper","targetId":"paper","labelSnapshot":"Title","content":[{"type":"text","text":"Title","styles":["bold","italic","underline"],"textColor":"blue","backgroundColor":"yellow"}]}]});
        validate_document(&document(content.clone())).expect("formatted label");
        let mut unsafe_color = content.clone();
        unsafe_color["items"][0]["content"][0]["textColor"] = serde_json::json!("url(evil)");
        assert!(validate_document(&document(unsafe_color)).is_err());
        let mut nested = content;
        nested["items"][0]["content"][0]["type"] = serde_json::json!("mention");
        assert!(validate_document(&document(nested)).is_err());
    }
}
