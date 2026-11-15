//! Core preview reads compared with the canonical Native decisions.
use super::*;
use crate::{
    ai::paper_reading_contracts::PaperContextRequest,
    docling::preparation::test_support::attachment,
};

#[tokio::test]
#[ignore = "requires actual independent literature package with preview capability"]
async fn actual_package_preview_matches_canonical_bytes_and_preserves_order_across_host_reopen() {
    let (catalog, key) = actual_package();
    let mut fixture = Fixture::new();
    attachment(&fixture.store, "paper", "selected", &"a".repeat(64));
    let raw = json!({
        "pages":{"1":{"page_no":1,"size":{"width":200.0,"height":400.0}}},
        "texts":[{"self_ref":"#/texts/0","text":"Authorized preview Ω","prov":[{"page_no":1,"bbox":{"l":20.0,"t":40.0,"r":120.0,"b":80.0}}]}],
        "tables":[{"self_ref":"#/tables/0","data":null,"prov":[{"page_no":1,"bbox":{"l":20.0,"t":100.0,"r":120.0,"b":140.0}}]}],
        "pictures":[{"self_ref":"#/pictures/0","label":"figure","prov":[{"page_no":1,"bbox":{"l":20.0,"t":160.0,"r":120.0,"b":240.0}}]}]
    });
    let (request, expected, changes) = {
        let mut db = fixture.store.connection().unwrap();
        db.execute("insert into docling_parse_runs(id,item_id,attachment_id,content_sha256,service_base_url,docling_status,parse_status,json_content,raw_response,created_at,finished_at) values('preview-parse','paper','selected',?1,'fixture','success','success',?2,'{}','now','now')", rusqlite::params!["a".repeat(64), raw.to_string()]).unwrap();
        let prepared = prepared_context(&db, "paper", &["selected".into()])
            .unwrap()
            .into_reading_result(None, None);
        let snapshot =
            paper_preparation::seed_snapshot(&mut db, "paper", &prepared, Default::default());
        let request = PaperContextRequest {
            paper_id: "paper".into(),
            snapshot_id: snapshot.id,
            expected_context_hash: snapshot.context_hash,
        };
        let expected =
            serde_json::to_vec(&paper_preparation::expected_preview(&db, &request)).unwrap();
        (request, expected, db.total_changes())
    };
    fixture.bind_actual_package(catalog.clone(), key.clone());
    assert!(fixture.store.paper_reading_source().is_none());
    let preview =
        crate::ai::paper_context::preview::preview_context(&fixture.store, request.clone())
            .await
            .unwrap();
    assert_eq!(serde_json::to_vec(&preview).unwrap(), expected);
    assert_eq!(preview.items.len(), 3);
    assert_eq!(
        preview
            .items
            .iter()
            .find(|item| item.item.kind == "table")
            .unwrap()
            .item
            .table_json,
        Some(Value::Null)
    );
    assert_eq!(fixture.store.connection().unwrap().total_changes(), changes);
    assert!(fixture.store.connection().unwrap().is_autocommit());
    assert_eq!(fixture.proofs(), 0);
    fixture.service.shutdown().await.unwrap();
    fixture.bind_actual_package(catalog, key.clone());
    let reopened =
        crate::ai::paper_context::preview::preview_context(&fixture.store, request.clone())
            .await
            .unwrap();
    assert_eq!(serde_json::to_vec(&reopened).unwrap(), expected);
    // Identity rejection precedes schema and budget reads, as on the static path.
    {
        let db = fixture.store.connection().unwrap();
        db.execute("update paper_context_snapshots set context_hash=?1, context_schema_version=2 where id=?2", rusqlite::params!["b".repeat(64), request.snapshot_id]).unwrap();
        db.execute_batch("drop table paper_context_items;").unwrap();
    }
    let error = crate::ai::paper_context::preview::preview_context(&fixture.store, request.clone())
        .await
        .unwrap_err();
    assert_eq!(error["code"], "conflict", "{error}");
    assert_eq!(error["nativeFailure"]["packageKey"], key);
    assert!(error["nativeFailure"]["nativeRequestId"].as_str().is_some());
    fixture
        .store
        .connection()
        .unwrap()
        .execute(
            "update bibliographic_items set trashed_at='now' where id='paper'",
            [],
        )
        .unwrap();
    let error = crate::ai::paper_context::preview::preview_context(&fixture.store, request)
        .await
        .unwrap_err();
    assert_eq!(error["code"], "unavailable", "{error}");
    assert!(fixture.store.connection().unwrap().is_autocommit());
    fixture.service.shutdown().await.unwrap();
}
