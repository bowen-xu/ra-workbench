use super::*;
use crate::notes::{NoteDocument, NoteDocumentBlock};
use serde_json::{Value, json};
fn block(kind: &str, content: Value) -> NoteDocumentBlock {
    NoteDocumentBlock {
        id: uuid::Uuid::new_v4().to_string(),
        block_type: kind.into(),
        props: json!({}),
        content,
        children: vec![],
    }
}
fn text(value: &str) -> Value {
    json!({"kind":"inline","items":[{"type":"text","text":value,"styles":[]}]})
}
fn document(blocks: Vec<NoteDocumentBlock>) -> NoteDocument {
    NoteDocument {
        title: None,
        schema_version: 1,
        page_id: uuid::Uuid::new_v4().to_string(),
        blocks,
    }
}
fn equals(left: &NoteDocument, right: &NoteDocument) {
    assert_eq!(
        serde_json::to_value(left).unwrap(),
        serde_json::to_value(right).unwrap()
    );
}

#[test]
fn preserves_every_block_family_and_hidden_properties_on_unchanged_projection() {
    let mut blocks = vec![];
    for kind in [
        "paragraph",
        "heading",
        "bulletListItem",
        "numberedListItem",
        "checkListItem",
        "quote",
        "codeBlock",
    ] {
        blocks.push(block(kind, text("原文 <xml> & 字符 🧪")));
    }
    blocks.extend([block("divider",json!({"kind":"none"})),block("formulaBlock",json!({"kind":"formula","source":"a < b"})),
        block("table",json!({"kind":"table","rows":[[text("a | b"),text("**literal**")],[text("ragged")]]})),
        block("unsupported",json!({"kind":"unsupported","originalType":"future","rawJson":"{\"secretFields\":true}"}))]);
    for kind in ["image", "file"] {
        blocks.push(block(kind,json!({"kind":"asset","assetId":"a".repeat(64),"displayName":"图 & 附件","mimeType":"image/png","caption":"说明"})));
    }
    for kind in [
        "childPage",
        "readingNoteReference",
        "researchObjectReference",
    ] {
        blocks.push(block(kind,json!({"kind":"reference","targetKind":"note_page","targetId":uuid::Uuid::new_v4().to_string(),"labelSnapshot":"名称","quoteSnapshot":"原文 ]]> & <"})));
    }
    blocks[0].props = json!({"future":"retained","textAlignment":"center"});
    blocks[0].children = vec![block("paragraph", text("子块"))];
    let before = document(blocks);
    let source = render(&before, "测试").unwrap();
    assert!(!source.contains(&before.blocks[0].id));
    assert!(!source.contains("secretFields"));
    let (title, after) = candidate(&before, &source).unwrap();
    assert_eq!(title, "测试");
    equals(&before, &after);
}

#[test]
fn distinguishes_hyperlinks_and_object_references_and_preserves_styles() {
    let before = document(vec![block(
        "paragraph",
        json!({"kind":"inline","items":[
            {"type":"text","text":"加粗 & 下划线","styles":["bold","underline"],"textColor":"red"},
            {"type":"link","href":"https://example.com?a=1&b=2","content":[{"type":"text","text":"网页","styles":[]}]},
            {"type":"mention","targetKind":"paper","targetId":"paper-id","labelSnapshot":"论文","content":[{"type":"text","text":"论文","styles":["italic"]}]}
        ]}),
    )]);
    let source = render(&before, "笔记").unwrap();
    let (_, after) = candidate(&before, &source).unwrap();
    equals(&before, &after);
    assert!(source.contains("<reference"));
    assert!(source.contains("target=\"source://library/paper-id\""));
    assert!(source.contains("[网页]"));
}

#[test]
fn all_object_references_and_links_use_source_addresses() {
    for (body, expected_id) in [
        (
            "<reference kind=\"paper\" target=\"source://library/paper-id\">Paper</reference>",
            "paper-id",
        ),
        (
            "<reference kind=\"paper\" target=\"source://library/paper-id\" display=\"block\" label=\"Paper\"><![CDATA[quote]]></reference>",
            "paper-id",
        ),
        (
            "<child-page target=\"source://notes/child-id\" label=\"Child\"/>",
            "child-id",
        ),
        (
            "<reference kind=\"note_page\" target=\"source://notes/%E7%A0%94%E7%A9%B6%2Fnote\">Note</reference>",
            "研究/note",
        ),
        ("[Paper](source://library/paper-id)", "paper-id"),
    ] {
        let (_, parsed) = parse("page", &format!("---\ntitle: Test\n---\n\n{body}\n")).unwrap();
        let rendered = render(&parsed, "Test").unwrap();
        assert!(rendered.contains("source://"), "{rendered}");
        let content = &parsed.blocks[0].content;
        let actual = if content["kind"] == "reference" {
            &content["targetId"]
        } else if content["items"][0]["type"] == "link" {
            assert_eq!(content["items"][0]["href"], "source://library/paper-id");
            continue;
        } else {
            &content["items"][0]["targetId"]
        };
        assert_eq!(actual, expected_id);
        equals(&parsed, &candidate(&parsed, &rendered).unwrap().1);
    }
}

#[test]
fn changes_text_without_replacing_unaffected_blocks_or_metadata() {
    let mut first = block("paragraph", text("old"));
    first.props = json!({"extra":"retain"});
    let before = document(vec![first, block("paragraph", text("unaffected"))]);
    let source = render(&before, "笔记").unwrap().replace("old", "new");
    let (_, after) = candidate(&before, &source).unwrap();
    assert_eq!(after.blocks[0].id, before.blocks[0].id);
    assert_eq!(after.blocks[0].props["extra"], "retain");
    assert_eq!(after.blocks[0].content["items"][0]["text"], "new");
    assert_eq!(
        serde_json::to_value(&after.blocks[1]).unwrap(),
        serde_json::to_value(&before.blocks[1]).unwrap()
    );
}

#[test]
fn rejects_arbitrary_xml_and_invalid_nesting_without_losing_the_original() {
    for body in [
        "<script>x</script>",
        "<file src=\"asset://a\" onclick=\"x\"/>",
        "<span color=\"invisible\">x</span>",
        "<u>unclosed",
        "<table><td>x</td></table>",
        "<children>x</children>",
        "<reference kind=\"paper\" target=\"source://library/paper-id/child-id\">x</reference>",
    ] {
        assert!(
            parse("p", &format!("---\ntitle: test\n---\n\n{body}\n")).is_err(),
            "{body}"
        );
    }
}
#[test]
fn ordinary_nested_lists_stay_markdown_and_code_math_keep_xml_literal() {
    let mut list = block("bulletListItem", text("parent"));
    list.children = vec![block("numberedListItem", text("child"))];
    let before = document(vec![
        list,
        block(
            "codeBlock",
            text("<reference kind=\"paper\">literal & raw</reference>"),
        ),
        block("formulaBlock", json!({"kind":"formula","source":"x < y"})),
    ]);
    let source = render(&before, "Test").unwrap();
    assert!(!source.contains("<children>"));
    let (_, parsed) = parse(&before.page_id, &source).unwrap();
    assert_eq!(render(&parsed, "Test").unwrap(), source);
    equals(&candidate(&before, &source).unwrap().1, &before);
}
#[test]
fn style_order_is_canonical_but_unmodified_native_order_is_retained() {
    let before = document(vec![block(
        "paragraph",
        json!({"kind":"inline","items":[{"type":"text","text":"格式","styles":["strike","bold","italic","underline"]}]}),
    )]);
    let source = render(&before, "Test").unwrap();
    let (_, parsed) = parse(&before.page_id, &source).unwrap();
    assert_eq!(render(&parsed, "Test").unwrap(), source);
    equals(&before, &candidate(&before, &source).unwrap().1);
}
#[test]
fn changing_parent_type_keeps_unchanged_children_and_text_edits_keep_container_metadata() {
    let mut heading = block("heading", text("title"));
    heading.props = json!({"level":2,"sidecar":"kept"});
    heading.children = vec![block("paragraph", text("child"))];
    let before = document(vec![heading]);
    let source = render(&before, "Test")
        .unwrap()
        .replace("## title", "title");
    let (_, after) = candidate(&before, &source).unwrap();
    assert_eq!(after.blocks[0].id, before.blocks[0].id);
    assert_eq!(after.blocks[0].props["sidecar"], "kept");
    assert_eq!(
        after.blocks[0].children[0].id,
        before.blocks[0].children[0].id
    );
    let mut paragraph = block("paragraph", text("old"));
    paragraph.content["sidecar"] = json!({"retained":true});
    let before = document(vec![paragraph]);
    let source = render(&before, "Test").unwrap().replace("old", "new");
    assert_eq!(
        candidate(&before, &source).unwrap().1.blocks[0].content["sidecar"],
        json!({"retained":true})
    );
}
#[test]
fn patch_ranges_preserve_repeated_blocks_outside_a_split_and_do_not_guess_moves() {
    let before = document(vec![
        block("paragraph", text("duplicate")),
        block("paragraph", text("unique")),
        block("paragraph", text("duplicate")),
    ]);
    let source = render(&before, "Test").unwrap();
    let hunk=research_agent_core::text_patch::parse("*** Begin Patch\n*** Update File: notes/root/p.md\n@@\n-unique\n+first\n+\n+second\n*** End Patch").unwrap().remove(0);
    let change = research_agent_core::text_patch::backend::prepare(
        &hunk,
        Some(research_agent_core::text_patch::backend::TextSnapshot {
            name: None,
            path: "notes/root/p.md".into(),
            text: source.clone(),
            version: "v".into(),
        }),
    )
    .unwrap();
    let (_, after) = mapped_candidate(
        &before,
        "Test",
        &source,
        change.after.as_ref().unwrap(),
        &change.edits,
    )
    .unwrap();
    assert_eq!(after.blocks.len(), 4);
    assert_eq!(after.blocks[0].id, before.blocks[0].id);
    assert_eq!(after.blocks[1].id, before.blocks[1].id);
    assert_eq!(after.blocks[3].id, before.blocks[2].id);
    assert_ne!(after.blocks[2].id, before.blocks[1].id);
}

#[path = "tests/math.rs"]
mod math;

#[path = "tests/boundaries.rs"]
mod boundaries;

#[path = "tests/toggles.rs"]
mod toggles;
