use super::*;
use crate::notes::agent_rewrite::tests::{block, edit, inline, source};
use serde_json::{Value, json};

#[test]
fn selected_rewrite_preserves_the_whole_page_and_all_unselected_structures() {
    let mut selected = block("selected");
    selected["children"] = json!([block("nested")]);
    let blocks = json!([
        block("before"), selected, block("after"),
        {"id":"child","type":"childPage","props":{},"content":{"kind":"reference","targetKind":"note_page","targetId":"child-page","labelSnapshot":"Child","quoteSnapshot":""},"children":[]},
        {"id":"asset","type":"image","props":{},"content":{"kind":"asset","assetId":"a".repeat(64),"displayName":"img.png","mimeType":"image/png","caption":"Keep"},"children":[]},
        {"id":"future","type":"unsupported","props":{},"content":{"kind":"unsupported","originalType":"future","rawJson":"{\"retain\":true}"},"children":[]}
    ]);
    let evidence = source(json!([blocks[1]]), Some("selected"));
    let full = source(blocks.clone(), None);
    let original_evidence = evidence.clone();
    let original_full = full.clone();
    let result = candidate(&evidence, &full, &[edit("nested"), edit("selected")]).unwrap();
    let mut expected = blocks;
    expected[1]["content"] = inline("Revised 中文 📝");
    expected[1]["children"][0]["content"] = inline("Revised 中文 📝");
    assert_eq!(serde_json::to_value(result.blocks).unwrap(), expected);
    assert_eq!(evidence, original_evidence);
    assert_eq!(full, original_full);
    assert!(candidate(&evidence, &full, &[edit("before")]).is_err());
    assert!(candidate(&evidence, &evidence, &[edit("selected")]).is_err());
}

#[test]
fn unchanged_revision_never_masks_changes_to_selected_text_props_or_children() {
    let blocks = json!([block("selected"), block("other")]);
    let evidence = source(json!([blocks[0]]), Some("selected"));
    for change in ["content", "props", "type", "children", "id"] {
        let mut changed = blocks.clone();
        match change {
            "content" => changed[0]["content"] = inline("User changed this"),
            "props" => changed[0]["props"] = json!({"alignment":"right"}),
            "type" => changed[0]["type"] = json!("quote"),
            "children" => changed[0]["children"] = json!([block("new-child")]),
            "id" => changed[0]["id"] = json!("replacement"),
            _ => unreachable!(),
        }
        assert!(
            candidate(&evidence, &source(changed, None), &[edit("selected")]).is_err(),
            "{change}"
        );
    }
}

#[test]
fn full_page_sources_do_not_ignore_unselected_sibling_changes() {
    let original = source(json!([block("selected"), block("other")]), None);
    let mut changed: Value = serde_json::from_str(&original.text).unwrap();
    changed["blocks"][1]["content"] = inline("User changed other block");
    let current = source(changed["blocks"].clone(), None);
    assert!(candidate(&original, &current, &[edit("selected")]).is_err());
    let result = candidate(&original, &original, &[edit("selected")]).unwrap();
    assert_eq!(result.blocks[1].content, inline("Original"));
}

#[test]
fn whole_page_limits_apply_after_a_valid_small_selection_rewrite() {
    let evidence = source(json!([block("selected")]), Some("selected"));
    let mut blocks = vec![block("selected")];
    for i in 0..4 {
        let mut sibling = block(&format!("large-{i}"));
        sibling["content"] = inline(&"x".repeat(1_999_000));
        blocks.push(sibling);
    }
    let full = source(json!(blocks), None);
    let replacement = InlineRewrite {
        block_id: "selected".into(),
        content: inline(&"y".repeat(400_000)),
    };
    assert!(
        crate::notes::agent_rewrite::preview(
            &evidence,
            &[InlineRewrite {
                block_id: replacement.block_id.clone(),
                content: replacement.content.clone()
            }]
        )
        .is_ok()
    );
    assert!(candidate(&evidence, &full, &[replacement]).is_err());
    assert!(candidate(&evidence, &full, &[edit("selected")]).is_ok());
}

#[test]
fn valid_but_different_page_title_or_revision_is_not_a_rebase() {
    use sha2::{Digest, Sha256};
    use uuid::Uuid;
    let original = source(json!([block("selected")]), None);
    for field in ["pageId", "title", "pageRevision"] {
        let mut current = original.clone();
        let mut raw: Value = serde_json::from_str(&current.text).unwrap();
        raw[field] = match field {
            "pageId" => json!(Uuid::new_v4()),
            "title" => json!("New title"),
            _ => json!(5),
        };
        current.text = raw.to_string();
        current.resource_id = raw["pageId"].as_str().unwrap().into();
        current.revision = format!(
            "revision:{}:sha256:{:x}",
            raw["pageRevision"],
            Sha256::digest(&current.text)
        );
        let page = Uuid::parse_str(&current.resource_id).unwrap();
        current.source_id = format!(
            "note:{:x}",
            Sha256::digest(serde_json::to_vec(&(page, Option::<String>::None)).unwrap())
        );
        current.anchor = Some(json!({"schemaVersion":1,"pageId":page,"pageRevision":raw["pageRevision"],"blockId":null}).to_string());
        assert!(crate::notes::agent_rewrite::preview(&current, &[edit("selected")]).is_ok());
        assert!(
            candidate(&original, &current, &[edit("selected")]).is_err(),
            "{field}"
        );
    }
}
