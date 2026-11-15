use super::*;
use serde_json::json;

pub(super) fn inline(text: &str) -> Value {
    json!({"kind":"inline","items":[{"type":"text","text":text,"styles":["bold"]}]})
}
pub(super) fn block(id: &str) -> Value {
    json!({"id":id,"type":"paragraph","props":{"alignment":"left"},"content":inline("Original"),"children":[]})
}
pub(super) fn source(blocks: Value, selected: Option<&str>) -> ContextText {
    let page = Uuid::from_u128(1);
    let text = json!({"schemaVersion":1,"pageId":page,"title":"Synthetic","pageRevision":4,"blocks":blocks}).to_string();
    ContextText {
        source_id: format!(
            "note:{:x}",
            Sha256::digest(serde_json::to_vec(&(page, selected)).unwrap())
        ),
        kind: ContextSourceKind::Note,
        resource_id: page.to_string(),
        revision: format!("revision:4:sha256:{:x}", Sha256::digest(&text)),
        anchor: Some(
            json!({"schemaVersion":1,"pageId":page,"pageRevision":4,"blockId":selected})
                .to_string(),
        ),
        text,
    }
}
pub(super) fn edit(id: &str) -> InlineRewrite {
    InlineRewrite {
        block_id: id.into(),
        content: inline("Revised 中文 📝"),
    }
}

#[test]
fn previews_nested_blocks_without_changing_evidence_structure_or_assets() {
    let mut parent = block("parent");
    parent["children"] = json!([block("nested"),
        {"id":"asset","type":"image","props":{},"content":{"kind":"asset","assetId":"a".repeat(64),"displayName":"image.png","mimeType":"image/png","caption":"Evidence"},"children":[]},
        {"id":"future","type":"unsupported","props":{},"content":{"kind":"unsupported","originalType":"future","rawJson":"{\"retain\":true}"},"children":[]}
    ]);
    let source = source(json!([parent]), Some("parent"));
    let original = source.clone();
    let result = preview(&source, &[edit("nested"), edit("parent")]).unwrap();
    assert_eq!(result.len(), 2);
    assert_eq!(result[0].before, inline("Original"));
    assert_eq!(result[0].after, inline("Revised 中文 📝"));
    assert_eq!(source, original);
    for id in ["outside", "asset", "future"] {
        assert!(preview(&source, &[edit(id)]).is_err());
    }
}

#[test]
fn keeps_reference_multiplicity_but_allows_rewording_and_reordering() {
    let mention = json!({"type":"mention","targetKind":"paper","targetId":"paper-1","labelSnapshot":"Paper","content":[{"type":"text","text":"Paper","styles":["bold","italic"],"textColor":"blue"}]});
    let link = json!({"type":"link","href":"https://example.org/evidence","content":[{"type":"text","text":"Evidence","styles":[]}]});
    let mut original = block("text");
    original["content"]["items"] = json!([mention, link, mention]);
    let source = source(json!([original]), None);
    let mut change = edit("text");
    let mut renamed = mention.clone();
    renamed["labelSnapshot"] = json!("Reworded title");
    change.content["items"] = json!([link, renamed, mention]);
    assert!(preview(&source, &[change]).is_ok());
    for items in [
        json!([mention, link]),
        json!([mention, mention, mention, link]),
        json!([mention,mention,{"type":"link","href":"https://example.org/other","content":[]}]),
    ] {
        let mut change = edit("text");
        change.content["items"] = items;
        assert!(preview(&source, &[change]).is_err());
    }
}

#[test]
fn refuses_lossy_unknown_fields_invalid_content_and_duplicate_or_empty_operations() {
    let source = source(json!([block("text")]), None);
    assert!(preview(&source, &[]).is_err());
    assert!(preview(&source, &[edit("text"), edit("text")]).is_err());
    let too_many: Vec<_> = (0..33).map(|_| edit("text")).collect();
    assert!(preview(&source, &too_many).is_err());
    for content in [
        inline("Original"),
        json!({"kind":"inline","items":[],"future":true}),
        json!({"kind":"inline","items":[{"type":"text","text":"x","styles":[],"future":true}]}),
        json!({"kind":"inline","items":[{"type":"text","text":"x","styles":["unknown"]}]}),
        json!({"kind":"inline","items":[{"type":"link","href":"javascript:alert(1)","content":[]}]}),
    ] {
        assert!(
            preview(
                &source,
                &[InlineRewrite {
                    block_id: "text".into(),
                    content
                }]
            )
            .is_err()
        );
    }
    let mut unknown = block("text");
    unknown["future"] = json!(true);
    assert!(
        preview(
            &super::tests::source(json!([unknown]), None),
            &[edit("text")]
        )
        .is_err()
    );
    let mut unknown = block("text");
    unknown["content"]["items"][0]["future"] = json!(true);
    assert!(
        preview(
            &super::tests::source(json!([unknown]), None),
            &[edit("text")]
        )
        .is_err()
    );
}

#[test]
fn rejects_corrupted_identity_and_selection_even_if_the_body_hash_is_recomputed() {
    let valid = source(json!([block("text")]), Some("text"));
    let mut corrupt = valid.clone();
    corrupt.text.push(' ');
    assert!(preview(&corrupt, &[edit("text")]).is_err());
    let mut corrupt = valid.clone();
    corrupt.resource_id = Uuid::new_v4().to_string();
    assert!(preview(&corrupt, &[edit("text")]).is_err());
    let mut corrupt = valid.clone();
    corrupt.source_id.push('0');
    assert!(preview(&corrupt, &[edit("text")]).is_err());
    let mut corrupt = valid.clone();
    corrupt.kind = ContextSourceKind::Manuscript;
    assert!(preview(&corrupt, &[edit("text")]).is_err());
    let mut corrupt = valid.clone();
    corrupt.anchor = Some("{}".into());
    assert!(preview(&corrupt, &[edit("text")]).is_err());
    assert!(
        preview(
            &source(json!([block("text"), block("outside")]), Some("text")),
            &[edit("outside")]
        )
        .is_err()
    );
    assert!(
        preview(
            &source(json!([block("text"), block("text")]), None),
            &[edit("text")]
        )
        .is_err()
    );
}
