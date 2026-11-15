//! Deterministic assembly/export regressions; no PDF or inference models needed.
use docling_core::{DoclingDocument, Node};
use docling_pdf::{
    assemble::{assemble_page, merge_continuations, stamp_page_no},
    layout::Region,
    pdfium_backend::{PdfPage, TextCell},
};
use serde_json::{Value, json};

fn page(number: usize, parts: &[(&str, [f32; 4])], rotation: u16) -> Vec<Node> {
    let cells = parts
        .iter()
        .map(|(text, [l, t, r, b])| TextCell {
            text: (*text).into(),
            l: *l,
            t: *t,
            r: *r,
            b: *b,
        })
        .collect();
    let mut page = PdfPage::from_cells(512., 768., 1., cells);
    page.rotation = rotation;
    let regions = parts
        .iter()
        .map(|(_, [l, t, r, b])| Region {
            label: "text",
            score: 0.99,
            l: *l,
            t: *t,
            r: *r,
            b: *b,
        })
        .collect();
    let (mut nodes, _) = assemble_page(&page, regions, &[], &[], None);
    stamp_page_no(&mut nodes, number);
    nodes
}

fn document(mut nodes: Vec<Node>) -> DoclingDocument {
    merge_continuations(&mut nodes);
    let mut document = DoclingDocument::new("text-provenance");
    document.nodes = nodes;
    document
}

fn assert_fragment(item: &Value, index: usize, page: usize, bbox: [f64; 4], span: [usize; 2]) {
    let prov = &item["prov"][index];
    assert_eq!(prov["page_no"], page);
    assert_eq!(
        prov["bbox"],
        json!({
            "l": bbox[0], "t": bbox[1], "r": bbox[2], "b": bbox[3], "coord_origin": "TOPLEFT",
        })
    );
    assert_eq!(prov["charspan"], json!(span));
}

#[test]
fn cross_page_continuation_keeps_each_page_box_and_unicode_character_range() {
    let first = "定义 x_α & an";
    let second = "anonymous instance";
    let third = "together with its type.";
    let mut nodes = page(447, &[(first, [40., 650., 460., 680.])], 0);
    nodes.extend(page(448, &[(second, [40., 30., 460., 60.])], 0));
    nodes.extend(page(449, &[(third, [40., 20., 460., 50.])], 0));
    let document = document(nodes);
    let result = document.export_to_json_value();
    assert_eq!(result["texts"].as_array().unwrap().len(), 1);
    let item = &result["texts"][0];
    assert_eq!(item["text"], format!("{first} {second} {third}"));
    assert_eq!(item["orig"], item["text"]);
    assert_eq!(item["prov"].as_array().unwrap().len(), 3);
    let a = first.chars().count();
    let b = a + 1 + second.chars().count();
    assert_fragment(item, 0, 447, [40., 650., 460., 680.], [0, a]);
    assert_fragment(item, 1, 448, [40., 30., 460., 60.], [a + 1, b]);
    assert_fragment(
        item,
        2,
        449,
        [40., 20., 460., 50.],
        [b + 1, b + 1 + third.chars().count()],
    );
    let mut output = Vec::new();
    document.write_json(&mut output).unwrap();
    assert_eq!(serde_json::from_slice::<Value>(&output).unwrap(), result);
    assert_eq!(document.export_to_markdown().matches(second).count(), 1);
    assert!(document.export_to_doclang().contains("<location"));
}

#[test]
fn same_page_column_merge_keeps_separate_boxes() {
    let nodes = page(
        448,
        &[
            ("An instance of", [32., 100., 220., 130.]),
            ("the current layer.", [280., 100., 480., 130.]),
        ],
        0,
    );
    let result = document(nodes).export_to_json_value();
    assert_eq!(result["texts"].as_array().unwrap().len(), 1);
    let item = &result["texts"][0];
    assert_eq!(item["text"], "An instance of the current layer.");
    assert_eq!(item["prov"].as_array().unwrap().len(), 2);
    assert_fragment(item, 0, 448, [32., 100., 220., 130.], [0, 14]);
    assert_fragment(item, 1, 448, [280., 100., 480., 130.], [15, 33]);
}

#[test]
fn column_merge_then_page_merge_retains_all_fragments() {
    let mut nodes = page(
        447,
        &[
            ("An instance of", [32., 100., 220., 130.]),
            ("the current", [280., 100., 480., 130.]),
        ],
        0,
    );
    nodes.extend(page(448, &[("layer.", [40., 30., 460., 60.])], 0));
    let result = document(nodes).export_to_json_value();
    let item = &result["texts"][0];
    assert_eq!(item["text"], "An instance of the current layer.");
    assert_eq!(item["prov"].as_array().unwrap().len(), 3);
    assert_fragment(item, 1, 447, [280., 100., 480., 130.], [15, 26]);
    assert_fragment(item, 2, 448, [40., 30., 460., 60.], [27, 33]);
}

#[test]
fn dehyphenation_adjusts_spans_instead_of_covering_the_removed_character() {
    let first = "中文 vocab-";
    let mut nodes = page(1, &[(first, [40., 650., 460., 680.])], 0);
    nodes.extend(page(2, &[("ulary β.", [40., 30., 460., 60.])], 0));
    let result = document(nodes).export_to_json_value();
    let item = &result["texts"][0];
    assert_eq!(item["text"], "中文 vocabulary β.");
    assert_fragment(item, 0, 1, [40., 650., 460., 680.], [0, 8]);
    assert_fragment(item, 1, 2, [40., 30., 460., 60.], [8, 16]);
}

#[test]
fn rotated_non_square_pages_keep_every_fragment_in_display_coordinates() {
    let nodes = page(
        448,
        &[
            ("An instance of", [32., 100., 220., 130.]),
            ("the current layer.", [280., 100., 480., 130.]),
        ],
        90,
    );
    let result = document(nodes).export_to_json_value();
    assert_eq!(
        result["pages"]["448"]["size"],
        json!({"width":768.,"height":512.})
    );
    let item = &result["texts"][0];
    assert_fragment(item, 0, 448, [638., 32., 668., 220.], [0, 14]);
    assert_fragment(item, 1, 448, [638., 280., 668., 480.], [15, 33]);
}

#[test]
fn complete_sentences_stay_separate_and_single_fragments_remain_locatable() {
    let mut nodes = page(1, &[("First sentence.", [40., 650., 460., 680.])], 0);
    nodes.extend(page(2, &[("another sentence.", [40., 30., 460., 60.])], 0));
    let result = document(nodes).export_to_json_value();
    assert_eq!(result["texts"].as_array().unwrap().len(), 2);
    assert_fragment(&result["texts"][1], 0, 2, [40., 30., 460., 60.], [0, 17]);
}
