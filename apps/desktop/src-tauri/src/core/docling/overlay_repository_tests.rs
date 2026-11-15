use super::*;
use crate::docling::{
    model::{DoclingParseRunDetail, DoclingParseRunSummary},
    overlay::build_page_overlay,
};
use serde_json::json;

fn connection(document: Option<&Value>) -> Connection {
    let db = Connection::open_in_memory().unwrap();
    db.execute_batch("create table docling_parse_runs(id text primary key,attachment_id text,parse_status text,json_content text,created_at text,finished_at text);").unwrap();
    db.execute("insert into docling_parse_runs values('run','attachment','success',?1,'2026-10-24','2026-10-24')",rusqlite::params![document.map(Value::to_string)]).unwrap();
    db
}
fn reference(document: Value, page: u32) -> DoclingPageOverlay {
    build_page_overlay(
        &DoclingParseRunDetail {
            summary: DoclingParseRunSummary {
                id: "run".into(),
                item_id: "paper".into(),
                attachment_id: "attachment".into(),
                content_sha256: "hash".into(),
                service_base_url: "synthetic".into(),
                docling_status: "success".into(),
                parse_status: "success".into(),
                error_message: None,
                processing_time_seconds: None,
                created_at: "2026-10-24".into(),
                finished_at: "2026-10-24".into(),
            },
            json_content: Some(document),
            raw_response: Value::Null,
        },
        "attachment",
        page,
    )
    .unwrap()
}
fn provenance(page: u32) -> Value {
    json!({"page_no":page,"bbox":{"l":10,"t":20,"r":80,"b":60,"coord_origin":"TOPLEFT"}})
}

#[test]
fn page_projection_matches_original_geometry_and_stable_item_references() {
    for pages in [
        json!({"2":{"page_no":2,"size":{"width":100,"height":200}},"1":{"page_no":1,"size":{"width":200,"height":400}}}),
        json!([{"page_no":1,"size":{"width":200,"height":400}},{"page_no":2,"size":{"width":100,"height":200}}]),
    ] {
        let mut doc = json!({"pages":pages});
        for (collection, _) in ITEM_COLLECTIONS {
            doc[collection] = json!([
                {"text":"other page","prov":[provenance(2)]},
                {"label":"paragraph","text":"multi page item","prov":[provenance(1),provenance(2)]},
                {"self_ref":"#/stable/independent","text":"stable ref","prov":[provenance(1)]},
                {"text":"no region","prov":"not an array"},
                null,
                {"prov":[null,"bad",{"page_no":"1"},{"page_no":1.5}]}
            ]);
        }
        let db = connection(Some(&doc));
        for page in [0, 1, 2, u32::MAX] {
            assert_eq!(
                read_page(&db, "attachment", page, Some("run")).unwrap(),
                reference(doc.clone(), page)
            );
        }
        assert_eq!(
            read_page(&db, "attachment", 0, None).unwrap().items[0].raw_ref,
            "#/texts/1"
        );
    }
}

#[test]
fn large_parse_returns_only_one_page_and_never_reads_audit_payload() {
    let pages = (1..=320)
        .map(|page| json!({"page_no":page,"size":{"width":100,"height":200}}))
        .collect::<Vec<_>>();
    let texts = (1..=320)
        .map(|page| json!({"text":"synthetic ".repeat(500),"prov":[provenance(page)]}))
        .collect::<Vec<_>>();
    let document = json!({"pages":pages,"texts":texts});
    assert!(document.to_string().len() > 1_500_000);
    let db = connection(Some(&document));
    // This isolated schema deliberately has no raw_response column: selecting
    // or hydrating it would fail, even with an empty/small audit payload.
    for page in [0, 159, 319] {
        let projected = page_document(&db, "run", page).unwrap();
        assert!(projected.to_string().len() < 6_000);
        assert_eq!(projected["texts"].as_array().unwrap().len(), 1);
        assert_eq!(
            read_page(&db, "attachment", page, None).unwrap(),
            reference(document.clone(), page)
        );
    }
}

#[test]
fn page_reads_preserve_owner_missing_document_and_parse_failures() {
    let db = connection(None);
    assert!(matches!(
        read_page(&db, "attachment", 0, Some("missing")),
        Err(DoclingError::ParseRunNotFound)
    ));
    assert!(matches!(
        read_page(&db, "other", 0, Some("run")),
        Err(DoclingError::Attachment(_))
    ));
    assert!(matches!(
        read_page(&db, "attachment", 0, Some("run")),
        Err(DoclingError::InvalidResponse(_))
    ));
    assert!(matches!(
        read_page(&db, "attachment", 0, None),
        Err(DoclingError::ParseRunNotFound)
    ));
    db.execute("update docling_parse_runs set json_content='invalid'", [])
        .unwrap();
    assert!(
        matches!(read_page(&db,"attachment",0,None),Err(DoclingError::InvalidResponse(message)) if message.contains("docling_overlay_invalid_json")&&message.contains("run=run"))
    );
}
