use super::*;

fn assert_round_trip(before: &NoteDocument) {
    let source = render(before, "边界").unwrap();
    let (title, parsed) = parse(&before.page_id, &source).unwrap();
    assert_eq!(render(&parsed, &title).unwrap(), source);
    // Unchanged authoring text must retain every original field and identity.
    let (_, preserved) = candidate(before, &source).unwrap();
    equals(before, &preserved);
}

#[test]
fn whitespace_survives_paragraph_formula_reference_and_table_boundaries() {
    for value in [
        " text",
        "text ",
        " ",
        "\ttext\t",
        "\ntext\n",
        "a\n\nb",
        "　text　",
    ] {
        let before = document(vec![block("paragraph", text(value))]);
        assert_round_trip(&before);
        let source = render(&before, "边界").unwrap();
        let (_, parsed) = parse(&before.page_id, &source).unwrap();
        assert_eq!(parsed.blocks[0].content["items"][0]["text"], value);
    }
    let before = document(vec![
        block(
            "paragraph",
            json!({"kind":"inline","items":[
                {"type":"text","text":" ","styles":[]},
                {"type":"inlineFormula","source":"x_1"},
                {"type":"text","text":" ","styles":[]}
            ]}),
        ),
        block(
            "paragraph",
            json!({"kind":"inline","items":[
                {"type":"mention","targetKind":"note_page","targetId":"target","labelSnapshot":"引用"},
                {"type":"text","text":" ","styles":[]}
            ]}),
        ),
        block(
            "table",
            json!({"kind":"table","rows":[[text(" cell ")],[text(" ")]]}),
        ),
    ]);
    assert_round_trip(&before);
}

#[test]
fn styled_punctuation_whitespace_and_neighbouring_runs_remain_exact() {
    for styles in [
        vec!["bold"],
        vec!["italic"],
        vec!["strike"],
        vec!["bold", "italic"],
        vec!["bold", "underline"],
    ] {
        for value in ["核心问题：", "（重点）", " 重点 ", " ", "a*b", "<文字>&"] {
            let before = document(vec![block(
                "paragraph",
                json!({"kind":"inline","items":[
                    {"type":"text","text":value,"styles":styles},
                    {"type":"text","text":"后文","styles":[]}
                ]}),
            )]);
            assert_round_trip(&before);
            let source = render(&before, "边界").unwrap();
            let (_, parsed) = parse(&before.page_id, &source).unwrap();
            assert_eq!(parsed.blocks[0].content["items"][0]["text"], value);
            for style in &styles {
                assert!(
                    parsed.blocks[0].content["items"][0]["styles"]
                        .as_array()
                        .unwrap()
                        .contains(&json!(style))
                );
            }
        }
    }
    for a in ["bold", "italic", "strike"] {
        for b in ["bold", "italic", "strike"] {
            let before = document(vec![block(
                "paragraph",
                json!({"kind":"inline","items":[
                    {"type":"text","text":"first","styles":[a]},
                    {"type":"text","text":"second","styles":[b]}
                ]}),
            )]);
            assert_round_trip(&before);
        }
    }
}

#[test]
fn boundary_safe_view_still_supports_local_patches_without_rewriting_other_blocks() {
    let before = document(vec![
        block(
            "paragraph",
            json!({"kind":"inline","items":[
                {"type":"text","text":"核心问题：","styles":["bold"]},
                {"type":"text","text":"待讨论 ","styles":[]}
            ]}),
        ),
        block("paragraph", text("untouched")),
    ]);
    let source = render(&before, "边界").unwrap();
    let (_, after) = candidate(&before, &source.replace("待讨论", "已讨论")).unwrap();
    assert_eq!(after.blocks[0].id, before.blocks[0].id);
    assert_eq!(
        serde_json::to_value(&after.blocks[1]).unwrap(),
        serde_json::to_value(&before.blocks[1]).unwrap()
    );
    assert_eq!(
        after.blocks[0].content["items"][0]["styles"],
        json!(["bold"])
    );
    assert_eq!(after.blocks[0].content["items"][1]["text"], "已讨论 ");
}

#[test]
fn text_boundary_matrix_preserves_visible_text_and_every_style_combination() {
    let names = ["bold", "italic", "strike", "underline", "code"];
    let values = [
        "word",
        "重点：",
        "（括号）",
        "*literal*",
        "a`b",
        "<tag>&",
        " ",
        "  padded  ",
        "",
        "🧪！",
        "e\u{301}",
    ];
    for bits in 0..32 {
        let styles = names
            .iter()
            .enumerate()
            .filter(|(i, _)| bits & (1 << i) != 0)
            .map(|(_, s)| *s)
            .collect::<Vec<_>>();
        for value in values {
            for (prefix, suffix) in [("", ""), ("前文", "后文"), ("*", "*"), (" ", " ")] {
                let before = document(vec![block(
                    "paragraph",
                    json!({"kind":"inline","items":[
                        {"type":"text","text":prefix,"styles":[]},
                        {"type":"text","text":value,"styles":styles},
                        {"type":"text","text":suffix,"styles":[]}
                    ]}),
                )]);
                let source = render(&before, "边界").unwrap();
                let (title, parsed) = parse(&before.page_id, &source).unwrap();
                assert_eq!(
                    render(&parsed, &title).unwrap(),
                    source,
                    "bits={bits} value={value:?} prefix={prefix:?} suffix={suffix:?}"
                );
                assert_round_trip(&before);
                // Canonical stability alone is insufficient: literal text and
                // the styles on every nonempty text run must also survive.
                let items = parsed.blocks[0].content["items"].as_array().unwrap();
                assert_eq!(
                    items
                        .iter()
                        .map(|i| i["text"].as_str().unwrap())
                        .collect::<String>(),
                    format!("{prefix}{value}{suffix}")
                );
                if !value.is_empty() && bits != 0 {
                    assert!(
                        items.iter().any(|i| i["text"].as_str() == Some(value)
                            && styles
                                .iter()
                                .all(|s| i["styles"].as_array().unwrap().contains(&json!(s)))),
                        "styles lost: {source}"
                    );
                }
            }
        }
    }
}

#[test]
fn styled_link_and_reference_labels_and_colours_preserve_boundaries() {
    for kind in ["link", "mention"] {
        for styles in [
            json!(["bold"]),
            json!(["italic", "underline"]),
            json!(["bold", "code"]),
        ] {
            let label = json!([{"type":"text","text":" 重点： ","styles":styles,"textColor":"red","backgroundColor":"yellow"}]);
            let item = if kind == "link" {
                json!({"type":"link","href":"https://example.com/?a=1&b=2","content":label})
            } else {
                json!({"type":"mention","targetKind":"note_page","targetId":"target","labelSnapshot":" 重点： ","content":label})
            };
            assert_round_trip(&document(vec![block(
                "paragraph",
                json!({"kind":"inline","items":[item]}),
            )]));
        }
    }
}

#[test]
fn adjacent_style_runs_and_nested_containers_remain_compositional() {
    assert_round_trip(&document(vec![block(
        "paragraph",
        json!({"kind":"inline","items":[
            {"type":"text","text":"word","styles":["bold","bold"]}
        ]}),
    )]));
    let names = ["bold", "italic", "strike", "underline", "code"];
    for a in 0..32 {
        for b in 0..32 {
            let styles = |bits| {
                names
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| bits & (1 << i) != 0)
                    .map(|(_, s)| *s)
                    .collect::<Vec<_>>()
            };
            let before = document(vec![block(
                "paragraph",
                json!({"kind":"inline","items":[
                    {"type":"text","text":"first","styles":styles(a)},
                    {"type":"text","text":"second","styles":styles(b)}
                ]}),
            )]);
            assert_round_trip(&before);
        }
    }
    for kind in [
        "heading",
        "quote",
        "bulletListItem",
        "numberedListItem",
        "checkListItem",
    ] {
        let mut parent = block(
            kind,
            json!({"kind":"inline","items":[
                {"type":"text","text":" 重点： ","styles":["bold","italic"]}
            ]}),
        );
        if kind == "heading" {
            parent.props = json!({"level":2});
        }
        parent.children = vec![block("paragraph", text(" child "))];
        assert_round_trip(&document(vec![parent]));
    }
}

#[test]
fn explicit_styles_do_not_expand_the_xml_permission_boundary() {
    for body in [
        "<strong onclick=\"run\">text</strong>",
        "<em color=\"red\">text</em>",
        "<s><image src=\"asset://bad\"/></s>",
        "<strong>\\(x\\)</strong>",
    ] {
        assert!(
            parse("p", &format!("---\ntitle: Test\n---\n\n{body}\n")).is_err(),
            "{body}"
        );
    }
    let before = document(vec![block(
        "paragraph",
        json!({"kind":"inline","items":[
            {"type":"text","text":"a\nb","styles":["code"]}
        ]}),
    )]);
    assert!(
        render(&before, "边界")
            .unwrap_err()
            .contains("multiline inline code")
    );
}
