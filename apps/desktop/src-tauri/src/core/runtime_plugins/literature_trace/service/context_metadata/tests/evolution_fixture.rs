//! Shared synthetic data from the original algorithm regression fixture.
use super::Fixture;
use rusqlite::{Connection, params};
pub(super) fn seed(fixture: &Fixture) {
    let connection = fixture.store.connection().unwrap();
    connection
        .execute_batch(
            "pragma foreign_keys = off;
             insert into bibliographic_items
             (id, item_type, title, issued_year, created_at, updated_at) values
             ('paper-1', 'journalArticle', 'Origin paper', 2020, 'now', 'now'),
             ('paper-2', 'journalArticle', 'Adaptation paper', 2022, 'now', 'now'),
             ('paper-3', 'journalArticle', 'Alternative paper', 2021, 'now', 'now');
             insert into concepts
             (id, label, explanation, disambiguation, domain, tradition, period,
              provenance_kind, revision, created_at, updated_at) values
             ('concept-shared', 'Shared foundation', 'Shared explanation', '', '', '', '',
              'human_original', 1, 'now', 'now'),
             ('concept-other', 'Other foundation', 'Other explanation', '', '', '', '',
              'human_original', 1, 'now', 'now');",
        )
        .expect("base fixtures");
    insert_assertion(
        &connection,
        "assertion-1",
        "paper-1",
        "concept-shared",
        "foundational",
        "introduced_by_paper",
        "Initial formulation",
    );
    insert_assertion(
        &connection,
        "assertion-2",
        "paper-2",
        "concept-shared",
        "foundational",
        "adapted_by_paper",
        "Later adaptation",
    );
    insert_assertion(
        &connection,
        "assertion-3",
        "paper-1",
        "concept-other",
        "foundational",
        "pre_existing",
        "Additional basis",
    );
    insert_assertion(
        &connection,
        "assertion-4",
        "paper-3",
        "concept-other",
        "foundational",
        "pre_existing",
        "Alternative basis",
    );
    connection
        .execute_batch("pragma foreign_keys = on;")
        .expect("foreign keys");
}
pub(super) fn insert_assertion(
    connection: &Connection,
    id: &str,
    paper_id: &str,
    concept_id: &str,
    paper_function: &str,
    novelty_status: &str,
    explanation: &str,
) {
    connection
        .execute(
            "insert into paper_concept_assertions
             (id, paper_id, concept_id, current_revision, created_at, updated_at)
             values (?1, ?2, ?3, 1, 'now', 'now')",
            params![id, paper_id, concept_id],
        )
        .expect("assertion");
    connection
        .execute(
            "insert into paper_concept_assertion_revisions
             (id, assertion_id, revision, paper_function, novelty_status, explanation_status,
              explanation, role_rationale, novelty_rationale, source_candidate_id,
              source_candidate_revision, source_local_concept_id, created_at)
             values (?1, ?2, 1, ?3, ?4, 'explained_in_paper', ?5, 'role', 'novelty',
                     ?6, 1, ?7, 'now')",
            params![
                format!("{id}-revision"),
                id,
                paper_function,
                novelty_status,
                explanation,
                format!("candidate-{id}"),
                format!("local-{id}"),
            ],
        )
        .expect("assertion revision");
}
