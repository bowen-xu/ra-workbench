use super::*;

#[test]
fn toggles_round_trip_rich_titles_nested_blocks_and_heading_mode() {
    let mut toggle = block("toggleListItem", text("Details < & ]]> 中文"));
    toggle.content["items"][0]["styles"] = json!(["bold"]);
    toggle.props = json!({"textColor":"green","textAlignment":"center","future":"keep"});
    let mut heading = block("heading", text("Nested heading"));
    heading.props = json!({"level":2,"isToggleable":true});
    heading.children = vec![block(
        "paragraph",
        json!({"kind":"inline","items":[
            {"type":"inlineFormula","source":"x_i^2"},
            {"type":"link","href":"https://example.org","content":[{"type":"text","text":"Link","styles":[]}]}
        ]}),
    )];
    toggle.children = vec![heading, block("toggleListItem", text(""))];
    let before = document(vec![toggle, block("quote", text("ordinary quote"))]);
    let source = render(&before, "Plan").unwrap();
    assert!(source.contains("<toggle>"));
    assert!(source.contains("toggle=\"true\""));
    assert!(source.contains("> ordinary quote"));
    let (_, unchanged) = candidate(&before, &source).unwrap();
    equals(&before, &unchanged);
    let (_, changed) = candidate(
        &before,
        &source.replace("Nested heading", "Updated heading"),
    )
    .unwrap();
    assert_eq!(
        changed.blocks[0].children[0].id,
        before.blocks[0].children[0].id
    );
    assert_eq!(changed.blocks[0].children[0].props["isToggleable"], true);
    assert_eq!(changed.blocks[0].props["future"], "keep");
    let (_, regular) = candidate(&before, &source.replace(" toggle=\"true\"", "")).unwrap();
    assert_ne!(regular.blocks[0].children[0].props["isToggleable"], true);
    assert_eq!(
        regular.blocks[0].children[0].id,
        before.blocks[0].children[0].id
    );
}

#[test]
fn admits_new_toggles_but_rejects_invalid_modes_and_attributes() {
    let before = document(vec![]);
    let source = "---\ntitle: \"Plan\"\n---\n\n<toggle><![CDATA[**Title**]]></toggle>\n\n<children><![CDATA[- item\n\n> quote]]></children>\n";
    let (_, after) = candidate(&before, source).unwrap();
    assert_eq!(after.blocks[0].block_type, "toggleListItem");
    assert_eq!(after.blocks[0].children[0].block_type, "bulletListItem");
    assert_eq!(after.blocks[0].children[1].block_type, "quote");
    crate::notes::document_validation::validate_document(&after).unwrap();
    for body in [
        "<toggle open=\"true\">title</toggle>",
        "text\n\n<block-format toggle=\"true\"/>",
        "# Heading\n\n<block-format toggle=\"maybe\"/>",
    ] {
        assert!(
            candidate(&before, &format!("---\ntitle: \"Plan\"\n---\n\n{body}\n")).is_err(),
            "{body}"
        );
    }
}
