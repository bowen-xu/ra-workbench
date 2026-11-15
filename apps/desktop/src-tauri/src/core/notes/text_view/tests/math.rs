use super::*;

fn formulas(blocks: &[NoteDocumentBlock], inline: &mut Vec<String>, display: &mut Vec<String>) {
    for block in blocks {
        if block.content["kind"] == "formula" {
            display.push(string(&block.content, "source").to_owned());
        }
        if let Some(items) = block.content["items"].as_array() {
            for item in items {
                if item["type"] == "inlineFormula" {
                    inline.push(string(item, "source").to_owned());
                }
            }
        }
        formulas(&block.children, inline, display);
    }
}

#[test]
fn shared_math_cases_parse_and_round_trip_as_native_formulas() {
    let cases: Vec<Value> = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../tests/fixtures/math-delimiters.json"
    )))
    .unwrap();
    for case in cases {
        let source = format!(
            "---\ntitle: Math\n---\n\n{}\n",
            case["source"].as_str().unwrap()
        );
        let (_, parsed) =
            parse("p", &source).unwrap_or_else(|error| panic!("{}: {error}", case["source"]));
        let mut inline = vec![];
        let mut display = vec![];
        formulas(&parsed.blocks, &mut inline, &mut display);
        assert_eq!(json!(inline), case["inline"], "{}", case["source"]);
        assert_eq!(json!(display), case["display"], "{}", case["source"]);
        let canonical = render(&parsed, "Math").unwrap();
        let (_, round_trip) = parse("p", &canonical).unwrap_or_else(|error| {
            panic!("case {} canonical {canonical}: {error}", case["source"])
        });
        assert_eq!(render(&round_trip, "Math").unwrap(), canonical);
    }
}

#[test]
fn xml_nested_math_and_literal_xml_inside_formulas_remain_distinct() {
    let (_, parsed) = parse(
        "p",
        r#"---
title: Math
---

<u>变量</u> \(x_i\)

<table><tr><td><![CDATA[\(a < b\)]]></td></tr></table>

\[
\text{<reference invalid="literal">} < x
\]
"#,
    )
    .unwrap();
    assert_eq!(
        parsed.blocks[0].content["items"]
            .as_array()
            .unwrap()
            .last()
            .unwrap()["type"],
        "inlineFormula"
    );
    assert_eq!(
        parsed.blocks[1].content["rows"][0][0]["items"][0]["source"],
        "a < b"
    );
    assert!(string(&parsed.blocks[2].content, "source").contains("<reference invalid="));
}

#[test]
fn formula_patch_uses_canonical_view_and_preserves_untouched_nodes() {
    let mut formula = block(
        "paragraph",
        json!({"kind":"inline","items":[
            {"type":"inlineFormula","source":"x_i","future":"retained"}
        ]}),
    );
    formula.props = json!({"future":"retained"});
    let before = document(vec![
        formula,
        block("formulaBlock", json!({"kind":"formula","source":"x < y"})),
    ]);
    let source = render(&before, "Math").unwrap();
    assert!(source.contains(r"\(x_i\)"));
    assert!(source.contains("\\[\nx < y\n\\]"));
    let (_, after) = candidate(&before, &source.replace("x < y", "x < z")).unwrap();
    equals(
        &document_with_blocks(&before, vec![before.blocks[0].clone()]),
        &document_with_blocks(&after, vec![after.blocks[0].clone()]),
    );
    assert_eq!(after.blocks[1].id, before.blocks[1].id);
    assert_eq!(after.blocks[1].content["source"], "x < z");
}

fn document_with_blocks(document: &NoteDocument, blocks: Vec<NoteDocumentBlock>) -> NoteDocument {
    NoteDocument {
        blocks,
        ..document.clone()
    }
}

#[test]
fn styled_links_round_trip_without_parsing_destination_as_math() {
    let before = document(vec![block(
        "paragraph",
        json!({"kind":"inline","items":[
            {"type":"link","href":"https://example.com/(id)","content":[
                {"type":"text","text":"链接","styles":["underline"]}
            ]}
        ]}),
    )]);
    let source = render(&before, "Links").unwrap();
    let (_, parsed) = parse(&before.page_id, &source).unwrap();
    assert_eq!(render(&parsed, "Links").unwrap(), source);
    let (_, parsed) = parse(
        "p",
        "---\ntitle: Links\n---\n\n[link](https://example.com/\\(id\\))\n",
    )
    .unwrap();
    assert_eq!(
        parsed.blocks[0].content["items"][0]["href"],
        "https://example.com/(id)"
    );
}
