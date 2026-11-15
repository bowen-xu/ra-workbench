use super::*;
use serde_json::json;

fn document(kind: &str, caption: Value) -> NoteDocument {
    NoteDocument {
        title: None,
        schema_version: 1,
        page_id: "caption-page".into(),
        blocks: vec![NoteDocumentBlock {
            id: "caption-block".into(),
            block_type: kind.into(),
            props: if kind == "table" {
                json!({"caption":caption,"textAlignment":"center"})
            } else {
                json!({"textAlignment":"right","previewWidth":273.5})
            },
            content: if kind == "table" {
                json!({"kind":"table","rows":[[{"kind":"inline","items":[{"type":"text","text":"A","styles":[]}]}]]})
            } else {
                json!({"kind":"asset","assetId":"a".repeat(64),"displayName":"image.png","mimeType":"image/png","caption":caption})
            },
            children: vec![],
        }],
    }
}

#[test]
fn captions_accept_four_thousand_utf16_units_including_cjk_and_surrogate_pairs() {
    for kind in ["image", "table"] {
        for caption in [
            "a".repeat(4_000),
            "文".repeat(4_000),
            "😀".repeat(2_000),
            "文\n".repeat(2_000),
        ] {
            assert_eq!(caption.encode_utf16().count(), 4_000);
            validate_document(&document(kind, json!(caption)))
                .expect("valid UTF-16 caption length");
        }
    }
}

#[test]
fn captions_reject_more_than_four_thousand_utf16_units_and_non_text_values() {
    for kind in ["image", "table"] {
        for caption in [
            "a".repeat(4_001),
            "文".repeat(4_001),
            "😀".repeat(2_000) + "a",
        ] {
            let error = validate_document(&document(kind, json!(caption)))
                .expect_err("caption exceeds limit");
            assert!(error.contains("说明") && error.contains("过长"), "{error}");
        }
        for caption in [
            json!(0),
            json!(true),
            Value::Null,
            json!({"text":"caption"}),
        ] {
            assert!(validate_document(&document(kind, caption)).is_err());
        }
    }
}

#[test]
fn caption_validation_preserves_literal_newlines_alignment_and_image_width() {
    for kind in ["image", "table"] {
        let document = document(kind, json!("第一行\n\nSecond line\r\n😀"));
        let before = serde_json::to_value(&document).expect("serialize document");
        validate_document(&document).expect("multiline caption");
        let reopened: NoteDocument =
            serde_json::from_value(before.clone()).expect("reopen document");
        assert_eq!(
            serde_json::to_value(reopened).expect("serialize reopened"),
            before
        );
    }
}

#[test]
fn legacy_table_caption_can_remain_absent() {
    let mut document = document("table", json!(""));
    document.blocks[0]
        .props
        .as_object_mut()
        .expect("props")
        .remove("caption");
    validate_document(&document).expect("legacy table");
    assert!(document.blocks[0].props.get("caption").is_none());
}

#[test]
fn captions_still_count_original_utf8_bytes_once_towards_the_document_budget() {
    for kind in ["image", "table"] {
        let mut document = document(kind, json!("文".repeat(4_000)));
        let block = document.blocks[0].clone();
        document.blocks = (0..350)
            .map(|index| NoteDocumentBlock {
                id: format!("caption-{index}"),
                ..block.clone()
            })
            .collect();
        validate_document(&document).expect("4.2 MB captions count once");
        document
            .blocks
            .extend((350..700).map(|index| NoteDocumentBlock {
                id: format!("caption-{index}"),
                ..block.clone()
            }));
        assert!(
            validate_document(&document)
                .expect_err("8.4 MB exceeds unchanged UTF-8 budget")
                .contains("文本总量")
        );
    }
}
