use super::*;
use serde_json::json;

fn document(asset_id: &str) -> NoteDocument {
    serde_json::from_value(json!({
        "schemaVersion": 1,
        "pageId": "projection",
        "blocks": [{
            "id": "image",
            "type": "image",
            "props": {},
            "content": {
                "kind": "asset",
                "assetId": asset_id,
                "displayName": "figure.png",
                "mimeType": "image/png",
                "caption": "Synthetic figure"
            },
            "children": []
        }]
    }))
    .unwrap()
}

#[test]
fn core_assets_require_content_hashes_while_external_analysis_retains_exact_document() {
    let uuid = "5051bdcd-c40b-48b6-b191-201f1fa30bc1";
    let original = document(uuid);
    assert_eq!(
        document_validation::validate_document(&original).unwrap_err(),
        "资产 ID 无效"
    );
    let external = analyze_with_external_assets(original.clone());
    assert_eq!(external.document, original);
    assert!(matches!(
        external.result,
        NoteDocumentAnalysisResult::Ready { text, .. } if text.contains("Synthetic figure")
    ));
    document_validation::validate_document(&document(&"a".repeat(64))).unwrap();
}

#[test]
fn external_assets_retain_structure_caption_and_field_limits() {
    let uuid = "5051bdcd-c40b-48b6-b191-201f1fa30bc1";
    for (key, invalid) in [
        ("displayName", json!("a".repeat(1_001))),
        ("mimeType", json!(false)),
        ("caption", json!("字".repeat(4_001))),
    ] {
        let mut input = document(uuid);
        input.blocks[0].content[key] = invalid;
        assert!(matches!(
            analyze_with_external_assets(input).result,
            NoteDocumentAnalysisResult::Invalid { .. }
        ));
    }
    let mut input = document(uuid);
    let duplicate = input.blocks[0].clone();
    input.blocks[0].children.push(duplicate);
    assert!(matches!(
        analyze_with_external_assets(input).result,
        NoteDocumentAnalysisResult::Invalid { cause } if cause == "笔记块 ID 为空、过长或重复"
    ));
}
