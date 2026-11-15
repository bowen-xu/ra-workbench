#[cfg(test)]
use super::{error::DoclingError, model::DoclingParseRunDetail};
use serde_json::Value;

use super::model::{
    DoclingOverlayItem, DoclingOverlayRect, DoclingOverlayRegion, DoclingPageOverlay,
    DoclingPageSize,
};

pub(super) const ITEM_COLLECTIONS: &[(&str, &str)] = &[
    ("texts", "text"),
    ("tables", "table"),
    ("pictures", "picture"),
    ("groups", "group"),
    ("key_value_items", "key_value"),
    ("form_items", "form"),
    ("field_items", "field"),
    ("field_regions", "field_region"),
];

#[cfg(test)]
pub(crate) fn build_page_overlay(
    run: &DoclingParseRunDetail,
    attachment_id: &str,
    page_index: u32,
) -> Result<DoclingPageOverlay, DoclingError> {
    if run.summary.attachment_id != attachment_id {
        return Err(DoclingError::Attachment(
            "Docling 解析记录不属于当前附件".to_owned(),
        ));
    }
    let document = run.json_content.as_ref().ok_or_else(|| {
        DoclingError::InvalidResponse("Docling 解析记录没有 json_content".to_owned())
    })?;
    Ok(build_document_overlay(
        &run.summary.id,
        attachment_id,
        page_index,
        document,
    ))
}

pub(super) fn build_document_overlay(
    run_id: &str,
    attachment_id: &str,
    page_index: u32,
    document: &Value,
) -> DoclingPageOverlay {
    let page_size = docling_page_size(document, page_index);
    let items = page_size
        .as_ref()
        .map(|size| docling_page_items(document, page_index, size))
        .unwrap_or_default();

    DoclingPageOverlay {
        attachment_id: attachment_id.to_owned(),
        parse_run_id: run_id.to_owned(),
        page_index,
        page_size,
        items,
    }
}

fn docling_page_items(
    document: &Value,
    page_index: u32,
    page_size: &DoclingPageSize,
) -> Vec<DoclingOverlayItem> {
    let mut items = Vec::new();
    for (collection_name, fallback_kind) in ITEM_COLLECTIONS {
        let Some(collection) = document.get(*collection_name).and_then(Value::as_array) else {
            continue;
        };
        for (index, item) in collection.iter().enumerate() {
            let regions = docling_item_regions(item, page_index, page_size);
            if regions.is_empty() {
                continue;
            }
            let raw_ref = item
                .get("self_ref")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
                .unwrap_or_else(|| format!("#/{collection_name}/{index}"));
            let label = item
                .get("label")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned);
            items.push(DoclingOverlayItem {
                id: raw_ref.clone(),
                kind: label.clone().unwrap_or_else(|| (*fallback_kind).to_owned()),
                label,
                text_preview: text_preview(item),
                regions,
                raw_ref,
            });
        }
    }
    items
}

fn docling_item_regions(
    item: &Value,
    page_index: u32,
    page_size: &DoclingPageSize,
) -> Vec<DoclingOverlayRegion> {
    let Some(provenance) = item.get("prov").and_then(Value::as_array) else {
        return Vec::new();
    };
    provenance
        .iter()
        .filter_map(|entry| {
            let page_no = entry.get("page_no").and_then(Value::as_u64)?;
            if page_no == 0 || page_no - 1 != u64::from(page_index) {
                return None;
            }
            let rect = normalize_docling_bbox(entry.get("bbox")?, page_size)?;
            Some(DoclingOverlayRegion { page_index, rect })
        })
        .collect()
}

fn docling_page_size(document: &Value, page_index: u32) -> Option<DoclingPageSize> {
    let page_no = u64::from(page_index) + 1;
    let pages = document.get("pages")?;
    if let Some(page) = pages.get(page_no.to_string()) {
        return page_size_from_value(page);
    }
    if let Some(pages) = pages.as_array() {
        return pages
            .iter()
            .find(|page| page.get("page_no").and_then(Value::as_u64) == Some(page_no))
            .and_then(page_size_from_value);
    }
    if let Some(pages) = pages.as_object() {
        return pages
            .values()
            .find(|page| page.get("page_no").and_then(Value::as_u64) == Some(page_no))
            .and_then(page_size_from_value);
    }
    None
}

fn page_size_from_value(page: &Value) -> Option<DoclingPageSize> {
    let size = page.get("size")?;
    let width = size.get("width").and_then(Value::as_f64)?;
    let height = size.get("height").and_then(Value::as_f64)?;
    if !width.is_finite() || !height.is_finite() || width <= 0.0 || height <= 0.0 {
        return None;
    }
    Some(DoclingPageSize { width, height })
}

fn normalize_docling_bbox(bbox: &Value, page_size: &DoclingPageSize) -> Option<DoclingOverlayRect> {
    let left = bbox.get("l").and_then(Value::as_f64)?;
    let top = bbox.get("t").and_then(Value::as_f64)?;
    let right = bbox.get("r").and_then(Value::as_f64)?;
    let bottom = bbox.get("b").and_then(Value::as_f64)?;
    if [left, top, right, bottom]
        .iter()
        .any(|value| !value.is_finite())
    {
        return None;
    }
    let x0 = left.min(right);
    let x1 = left.max(right);
    let y0 = top.min(bottom);
    let y1 = top.max(bottom);
    if x1 <= x0 || y1 <= y0 {
        return None;
    }
    let coord_origin = bbox
        .get("coord_origin")
        .and_then(Value::as_str)
        .unwrap_or("TOPLEFT");
    let normalized_y = if coord_origin == "BOTTOMLEFT" {
        (page_size.height - y1) / page_size.height
    } else {
        y0 / page_size.height
    };
    clamp_rect(DoclingOverlayRect {
        x: x0 / page_size.width,
        y: normalized_y,
        width: (x1 - x0) / page_size.width,
        height: (y1 - y0) / page_size.height,
    })
}

fn clamp_rect(rect: DoclingOverlayRect) -> Option<DoclingOverlayRect> {
    let x0 = rect.x.clamp(0.0, 1.0);
    let y0 = rect.y.clamp(0.0, 1.0);
    let x1 = (rect.x + rect.width).clamp(0.0, 1.0);
    let y1 = (rect.y + rect.height).clamp(0.0, 1.0);
    if x1 <= x0 || y1 <= y0 {
        return None;
    }
    Some(DoclingOverlayRect {
        x: x0,
        y: y0,
        width: x1 - x0,
        height: y1 - y0,
    })
}

fn text_preview(item: &Value) -> Option<String> {
    let value = item
        .get("text")
        .or_else(|| item.get("orig"))
        .and_then(Value::as_str)
        .or_else(|| {
            item.get("data")
                .and_then(|data| data.get("text"))
                .and_then(Value::as_str)
        })?;
    let normalized = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.is_empty() {
        None
    } else {
        Some(normalized.chars().take(160).collect())
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::docling::model::DoclingParseRunSummary;

    fn assert_rect_close(actual: &DoclingOverlayRect, expected: DoclingOverlayRect) {
        let epsilon = 0.000_001;
        assert!((actual.x - expected.x).abs() < epsilon);
        assert!((actual.y - expected.y).abs() < epsilon);
        assert!((actual.width - expected.width).abs() < epsilon);
        assert!((actual.height - expected.height).abs() < epsilon);
    }

    fn run(json_content: Value) -> DoclingParseRunDetail {
        DoclingParseRunDetail {
            summary: DoclingParseRunSummary {
                id: "run-1".to_owned(),
                item_id: "paper-1".to_owned(),
                attachment_id: "att-1".to_owned(),
                content_sha256: "a".repeat(64),
                service_base_url: "http://127.0.0.1:5001".to_owned(),
                docling_status: "success".to_owned(),
                parse_status: "success".to_owned(),
                error_message: None,
                processing_time_seconds: None,
                created_at: "2026-08-09T00:00:00.000Z".to_owned(),
                finished_at: "2026-08-09T00:00:00.000Z".to_owned(),
            },
            json_content: Some(json_content),
            raw_response: json!({ "status": "success" }),
        }
    }

    #[test]
    fn builds_page_overlay_from_text_table_and_picture_provenance() {
        let overlay = build_page_overlay(
            &run(json!({
                "pages": {
                    "1": { "page_no": 1, "size": { "width": 200.0, "height": 400.0 } }
                },
                "texts": [{
                    "self_ref": "#/texts/0",
                    "label": "paragraph",
                    "text": "  A paragraph   with spaces. ",
                    "prov": [{ "page_no": 1, "bbox": { "l": 20.0, "t": 40.0, "r": 120.0, "b": 80.0, "coord_origin": "TOPLEFT" }, "charspan": [0, 10] }]
                }],
                "tables": [{
                    "self_ref": "#/tables/0",
                    "label": "table",
                    "prov": [{ "page_no": 1, "bbox": { "l": 10.0, "t": 100.0, "r": 190.0, "b": 180.0, "coord_origin": "TOPLEFT" }, "charspan": [0, 1] }]
                }],
                "pictures": [{
                    "self_ref": "#/pictures/0",
                    "label": "picture",
                    "prov": [{ "page_no": 1, "bbox": { "l": 50.0, "t": 200.0, "r": 150.0, "b": 300.0, "coord_origin": "TOPLEFT" }, "charspan": [0, 1] }]
                }]
            })),
            "att-1",
            0,
        )
        .expect("build overlay");

        assert_eq!(
            overlay.page_size,
            Some(DoclingPageSize {
                width: 200.0,
                height: 400.0
            })
        );
        assert_eq!(overlay.items.len(), 3);
        assert_eq!(
            overlay.items[0].text_preview.as_deref(),
            Some("A paragraph with spaces.")
        );
        assert_rect_close(
            &overlay.items[0].regions[0].rect,
            DoclingOverlayRect {
                x: 0.1,
                y: 0.1,
                width: 0.5,
                height: 0.1,
            },
        );
    }

    #[test]
    fn converts_bottom_left_coordinates_to_top_left_normalized_rects() {
        let overlay = build_page_overlay(
            &run(json!({
                "pages": [{ "page_no": 1, "size": { "width": 200.0, "height": 400.0 } }],
                "texts": [{
                    "self_ref": "#/texts/0",
                    "text": "bottom-left",
                    "prov": [{ "page_no": 1, "bbox": { "l": 20.0, "t": 40.0, "r": 120.0, "b": 80.0, "coord_origin": "BOTTOMLEFT" }, "charspan": [0, 10] }]
                }]
            })),
            "att-1",
            0,
        )
        .expect("build overlay");

        assert_rect_close(
            &overlay.items[0].regions[0].rect,
            DoclingOverlayRect {
                x: 0.1,
                y: 0.8,
                width: 0.5,
                height: 0.1,
            },
        );
    }

    #[test]
    fn merged_paragraph_is_locatable_on_each_page_and_column() {
        let parsed = run(json!({
            "pages": {
                "447": { "page_no": 447, "size": { "width": 512., "height": 768. } },
                "448": { "page_no": 448, "size": { "width": 512., "height": 768. } }
            },
            "texts": [{ "self_ref": "#/texts/0", "label": "text", "text": "continued paragraph",
                "prov": [
                    { "page_no": 447, "charspan": [0, 4], "bbox": { "l": 40., "t": 650., "r": 460., "b": 680., "coord_origin": "TOPLEFT" } },
                    { "page_no": 448, "charspan": [5, 10], "bbox": { "l": 32., "t": 30., "r": 220., "b": 60., "coord_origin": "TOPLEFT" } },
                    { "page_no": 448, "charspan": [11, 19], "bbox": { "l": 280., "t": 30., "r": 480., "b": 60., "coord_origin": "TOPLEFT" } }
                ]
            }]
        }));
        let previous = build_page_overlay(&parsed, "att-1", 446).expect("previous page");
        let current = build_page_overlay(&parsed, "att-1", 447).expect("current page");
        assert_eq!(previous.items.len(), 1);
        assert_eq!(current.items.len(), 1);
        assert_eq!(previous.items[0].regions.len(), 1);
        assert_eq!(current.items[0].regions.len(), 2);
        assert_eq!(previous.items[0].raw_ref, current.items[0].raw_ref);
        assert_rect_close(
            &current.items[0].regions[1].rect,
            DoclingOverlayRect {
                x: 280. / 512.,
                y: 30. / 768.,
                width: 200. / 512.,
                height: 30. / 768.,
            },
        );
    }

    #[test]
    fn untranscribed_formulas_keep_overlay_regions_even_without_text() {
        for original in ["x²", ""] {
            let overlay = build_page_overlay(
                &run(json!({
                    "pages": { "448": { "page_no": 448, "size": { "width": 512., "height": 512. } } },
                    "texts": [{
                        "self_ref": "#/texts/0", "label": "formula", "text": "", "orig": original,
                        "prov": [{ "page_no": 448, "charspan": [0, 0],
                            "bbox": { "l": 64., "t": 412., "r": 400., "b": 392., "coord_origin": "BOTTOMLEFT" } }]
                    }]
                })),
                "att-1",
                447,
            ).expect("formula overlay");
            assert_eq!(overlay.items.len(), 1);
            assert_eq!(overlay.items[0].kind, "formula");
            assert_eq!(overlay.items[0].regions[0].page_index, 447);
            assert_rect_close(
                &overlay.items[0].regions[0].rect,
                DoclingOverlayRect {
                    x: 0.125,
                    y: 100. / 512.,
                    width: 336. / 512.,
                    height: 20. / 512.,
                },
            );
        }
    }

    #[test]
    fn skips_items_without_page_size_or_valid_provenance() {
        let overlay = build_page_overlay(
            &run(json!({
                "pages": { "1": { "page_no": 1, "size": { "width": 0.0, "height": 400.0 } } },
                "texts": [{
                    "self_ref": "#/texts/0",
                    "text": "invalid",
                    "prov": [{ "page_no": 1, "bbox": { "l": 20.0, "t": 40.0, "r": 120.0, "b": 80.0 }, "charspan": [0, 10] }]
                }]
            })),
            "att-1",
            0,
        )
        .expect("build overlay");

        assert_eq!(overlay.page_size, None);
        assert!(overlay.items.is_empty());
    }
}
