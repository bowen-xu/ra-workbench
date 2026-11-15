//! Assembly/export regression only: no models, original PDF or network required.
use docling_core::DoclingDocument;
use docling_pdf::{
    assemble::{Enrichment, assemble_page, stamp_page_no},
    layout::Region,
    pdfium_backend::{PdfPage, TextCell},
};
use serde_json::{Value, json};

fn formula_json(text: Option<&str>, latex: Option<&str>, rotation: u16) -> Value {
    let cells = text
        .map(|text| TextCell {
            text: text.into(),
            l: 64.,
            t: 100.,
            r: 400.,
            b: 120.,
        })
        .into_iter()
        .collect();
    let mut page = PdfPage::from_cells(512., 512., 1., cells);
    page.rotation = rotation;
    let regions = vec![Region {
        label: "formula",
        score: 0.95,
        l: 64.,
        t: 100.,
        r: 400.,
        b: 120.,
    }];
    let enrichment = vec![latex.map(|latex| Enrichment::Formula {
        latex: latex.into(),
    })];
    let (mut nodes, _) = assemble_page(&page, regions, &[], &enrichment, None);
    // Preserve the actual page, independent of local selection/page count.
    stamp_page_no(&mut nodes, 448);
    let mut document = DoclingDocument::new("formula-regression");
    document.nodes = nodes;
    document.export_to_json_value()
}

#[test]
fn detected_formula_retains_type_original_text_and_provenance_without_transcription() {
    let raw = "Pᵣ → [Pᵢ](pⱼ) ⟨f;c⟩ (2)";
    let document = formula_json(Some(raw), None, 0);
    let item = &document["texts"][0];
    assert_eq!(item["label"], "formula");
    assert_eq!(item["text"], "");
    assert_eq!(item["orig"], raw);
    assert_eq!(item["prov"][0]["page_no"], 448);
    assert_eq!(item["prov"][0]["charspan"], json!([0, 0]));
    assert_eq!(
        item["prov"][0]["bbox"],
        json!({"l":64.,"t":412.,"r":400.,"b":392.,"coord_origin":"BOTTOMLEFT"})
    );
    assert!(!document.to_string().contains("formula-not-decoded"));
}

#[test]
fn detected_formula_with_no_extractable_text_still_has_a_locatable_region() {
    let document = formula_json(None, None, 0);
    let item = &document["texts"][0];
    assert_eq!(item["label"], "formula");
    assert_eq!(item["orig"], "");
    assert_eq!(item["prov"][0]["page_no"], 448);
    assert!(item["prov"][0]["bbox"]["l"].is_number());
}

#[test]
fn transcribed_formulas_keep_latex_and_original_text() {
    let document = formula_json(Some("x²"), Some("x^2"), 0);
    let item = &document["texts"][0];
    assert_eq!(item["label"], "formula");
    assert_eq!(item["text"], "x^2");
    assert_eq!(item["orig"], "x²");
    assert_eq!(item["prov"][0]["charspan"], json!([0, 3]));
}

#[test]
fn untranscribed_formula_geometry_follows_page_rotation() {
    let document = formula_json(Some("x²"), None, 90);
    let item = &document["texts"][0];
    assert_eq!(item["label"], "formula");
    assert_eq!(
        item["prov"][0]["bbox"],
        json!({"l":391.,"t":448.,"r":411.,"b":112.,"coord_origin":"BOTTOMLEFT"})
    );
}
