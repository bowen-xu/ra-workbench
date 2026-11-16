//! Fixed contract inputs for Core transaction and controlled-wait tests.

mod sql;
pub(super) use sql::KNOWLEDGE_SQL;

pub(super) fn install_knowledge_sql(directory: &std::path::Path) {
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../tests/fixtures/native-resources-v1/literature");
    for file in [
        "concepts-search.v1.sql",
        "concepts-name-active.v1.sql",
        "concepts-merged.v1.sql",
        "concepts-current-revision.v1.sql",
        "concepts-alias-update.v1.sql",
        "concepts-preferred-upsert.v1.sql",
        "concepts-update.v1.sql",
        "nal-bootstrap-term-read.v1.sql",
        "nal-bootstrap-term-lookup.v1.sql",
        "nal-bootstrap-term-insert.v1.sql",
        "nal-bootstrap-concept-main.v1.sql",
        "nal-bootstrap-concept-terms.v1.sql",
        "nal-bootstrap-concept-insert.v1.sql",
        "nal-bootstrap-concept-preferred-insert.v1.sql",
        "nal-bootstrap-concept-name-available.v1.sql",
        "nal-bootstrap-research-revision-insert.v1.sql",
        "nal-bootstrap-representation-lookup.v1.sql",
        "nal-bootstrap-semantic-old-jobs-cancel.v1.sql",
        "nal-bootstrap-semantic-old-records-tombstone.v1.sql",
        "nal-bootstrap-representation-insert.v1.sql",
        "nal-bootstrap-embedding-space-insert.v1.sql",
        "nal-bootstrap-generation-active.v1.sql",
        "nal-bootstrap-generation-next.v1.sql",
        "nal-bootstrap-generation-insert.v1.sql",
        "nal-bootstrap-embedding-job-lookup.v1.sql",
        "nal-bootstrap-embedding-record-insert.v1.sql",
        "nal-bootstrap-embedding-job-insert.v1.sql",
        "duplicate-merge-legacy-preferred-term-id.v1.sql",
        "duplicate-merge-legacy-render-concept.v1.sql",
        "duplicate-merge-semantic-space-policy.v1.sql",
        "duplicate-merge-semantic-generation-path.v1.sql",
        "duplicate-merge-semantic-record-row-id.v1.sql",
        "duplicate-merge-semantic-job-idempotency.v1.sql",
    ] {
        std::fs::copy(source.join(file), directory.join("data").join(file)).unwrap();
    }
    for &(name, sql) in KNOWLEDGE_SQL {
        std::fs::write(directory.join("data").join(name), sql).unwrap();
    }
}

pub(super) fn knowledge(
    group: &str,
    input: &research_workbench_native_sdk::knowledge_promotion::KnowledgeValidationInputV1,
) -> research_workbench_native_sdk::knowledge_promotion::KnowledgeValidationDecisionV1 {
    let mut observed = serde_json::to_value(input).unwrap();
    // The policy response is independent of Core's opaque read-set identities.
    for field in [
        "snapshotSha256",
        "subjectSha256",
        "objectSha256",
        "currentSha256",
        "conceptIdentitySha256",
    ] {
        observed.as_object_mut().unwrap().remove(field);
    }
    let fixtures: serde_json::Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../tests/fixtures/knowledge-responses-v1/decisions.json"
    )))
    .unwrap();
    let response = fixtures[group]
        .as_array()
        .expect("known controlled-wait group")
        .iter()
        .find(|row| row["input"] == observed)
        .unwrap_or_else(|| panic!("unrecorded controlled-wait input in {group}: {observed}"));
    serde_json::from_value(response["decision"].clone()).unwrap()
}
