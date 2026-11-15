use super::*;
use serde_json::json;

#[path = "host_tests.rs"]
mod host_tests;

fn item(id: &str) -> CatalogItem {
    serde_json::from_value(json!({
        "id":id,"itemType":"journalArticle","title":"Synthetic title",
        "abstractText":"Synthetic abstract","issuedYear":2026,"containerTitle":"Journal",
        "conferenceName":"","proceedingsTitle":"","place":"","volume":"1",
        "issue":"2","pages":"3–4","doi":"10.synthetic/example","url":"",
        "language":"zh","bibtexKey":"synthetic","rawBibtex":"@article{synthetic}",
        "readingStatus":"unread",
        "creators":[{"familyName":"Example","givenName":"A","literalName":"","role":"author"}],
        "tags":["synthetic"],
        "attachments":[{"id":"attachment","displayName":"synthetic.pdf",
            "mimeType":"application/pdf","byteSize":123,"contentSha256":"synthetic-hash"}],
        "projectIds":["project"],
        "metadataSources":[{"fieldName":"title","sourceKind":"manual","confirmedByUser":true}],
        "trashedAt":null,"createdAt":"synthetic-created","updatedAt":"synthetic-updated"
    }))
    .unwrap()
}

fn success(ids: &[&str]) -> CatalogProjectionOutcome {
    CatalogProjectionOutcome::Success {
        items: ids.iter().map(|id| item(id)).collect(),
    }
}

fn ids(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

#[test]
fn get_success_requires_the_requested_identity_as_well_as_exactly_one_item() {
    let expected = ids(&["requested"]);
    assert!(projection_ids_match(&success(&["requested"]), &expected));
    for returned in [vec!["different"], vec![], vec!["requested", "different"]] {
        assert!(!projection_ids_match(&success(&returned), &expected));
    }
}

#[test]
fn list_success_rejects_missing_extra_reordered_or_deduplicated_capture_targets() {
    let expected = ids(&["a", "b", "a"]);
    for returned in [
        vec![],
        vec!["a"],
        vec!["a", "b"],
        vec!["a", "b", "a", "c"],
        vec!["b", "a", "a"],
        vec!["a", "a", "b"],
        vec!["a", "b", "c"],
    ] {
        assert!(!projection_ids_match(&success(&returned), &expected));
    }
}

#[test]
fn exact_order_duplicates_empty_results_and_raw_identity_bytes_are_preserved() {
    let raw = ["  原始\0 ID  ", "second", "  原始\0 ID  "];
    let mut outcome = success(&raw);
    if let CatalogProjectionOutcome::Success { items } = &mut outcome {
        // This check owns identity binding, not domain field policy or projection.
        items[0].reading_status = "original-open-domain-value".into();
        items[0].creators[0].role = "original-open-role".into();
    }
    let before = outcome.clone();
    assert!(projection_ids_match(&outcome, &ids(&raw)));
    assert_eq!(outcome, before);
    assert!(projection_ids_match(&success(&[]), &[]));
    assert!(!projection_ids_match(&success(&["extra"]), &[]));
}

#[test]
fn not_found_passes_identity_check_without_becoming_an_empty_success() {
    let outcome = CatalogProjectionOutcome::NotFound;
    assert!(projection_ids_match(&outcome, &[]));
    assert!(projection_ids_match(&outcome, &ids(&["missing"])));
    assert!(projection_ids_match(&outcome, &ids(&["a", "missing", "a"])));
    assert_eq!(outcome, CatalogProjectionOutcome::NotFound);
}
