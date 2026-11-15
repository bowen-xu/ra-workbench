//! Real seven-variant mutation checks; fixture effects stay in this synthetic CLI.
use super::*;
use research_workbench_native_sdk::{
    nal_annotation::NalAnnotation,
    nal_mutation::NalMutation,
    nal_state::{NalState, NalStatement, NalTerm},
};
use serde_json::Value;
use std::sync::atomic::Ordering;

#[path = "nal_mutation_check/reopen.rs"]
mod cold;
#[path = "nal_mutation_check/failures.rs"]
mod failures;
use super::{nal_bootstrap_fixture as fixture, nal_bootstrap_readback as readback};

type Error = Box<dyn std::error::Error>;
const ENDPOINT: &str = "bootstrap-endpoint";
const RETAINED_ANNOTATION: &str = "mutation-retained-note";

fn term(id: &str) -> NalTerm {
    NalTerm::Concept { id: id.into() }
}
fn paper() -> NalTerm {
    NalTerm::Paper {
        id: "bootstrap-paper".into(),
    }
}
fn product(terms: Vec<NalTerm>) -> NalTerm {
    NalTerm::Product { terms }
}
fn statement(id: &str, subject: NalTerm, predicate: NalTerm) -> NalStatement {
    NalStatement {
        id: id.into(),
        subject,
        predicate,
        origin: "human".into(),
    }
}
fn annotation(id: &str, kind: &str, target: &str, note: &str) -> NalAnnotation {
    NalAnnotation {
        id: id.into(),
        target_kind: kind.into(),
        target_id: target.into(),
        note_page_id: note.into(),
    }
}
async fn read(
    store: &AppStore,
    service: &Arc<LiteratureTraceService>,
    key: &str,
) -> Result<NalState, Error> {
    Ok(service
        .read_nal_bootstrap(store.shared_agent_connection(), key.into())
        .await?)
}

struct Checks<'a> {
    store: &'a AppStore,
    service: &'a Arc<LiteratureTraceService>,
    key: &'a str,
    note: &'a str,
    document: &'a Value,
    state: NalState,
    steps: Vec<Value>,
}
impl Checks<'_> {
    async fn apply(&mut self, name: &str, mutation: NalMutation, delta: i64) -> Result<(), Error> {
        let before = self.state.revision;
        let state = self
            .service
            .mutate_nal_network(
                self.store.shared_agent_connection(),
                self.key.into(),
                mutation.clone(),
                before,
            )
            .await?;
        // Original mutate bumps once. The deployed nal_concept_updated trigger
        // adds one more only for this scenario's actual concept soft delete.
        if state.revision != before + delta
            || fixture::note(self.store, self.note)? != *self.document
            || readback::scalar(self.store, "select count(*) from note_pages")? != 1
        {
            return Err(format!(
                "synthetic mutation {name} changed its original revision/Core Note contract"
            )
            .into());
        }
        self.steps.push(
            json!({"name":name,"mutation":mutation,"beforeRevision":before,
            "afterRevision":state.revision,"expectedDelta":delta,"sameCoreNote":true}),
        );
        self.state = state;
        Ok(())
    }
}

pub(super) async fn run(
    store: &AppStore,
    service: &Arc<LiteratureTraceService>,
    key: &str,
) -> Result<Value, Error> {
    let startup = failures::startup(store, service, key).await?;
    let survivor = startup
        .state
        .concepts
        .iter()
        .find(|c| c.label == "Synthetic collection B")
        .ok_or("synthetic imported survivor missing")?
        .id
        .clone();
    let mut fault_results = startup.failures;
    fault_results
        .extend(failures::ordered_and_final(store, service, key, &startup.state, &survivor).await?);
    if fixture::note(store, &startup.note)? != startup.document
        || readback::metadata(store)? != startup.metadata
    {
        return Err(
            "failed mutations changed the actual SQL metadata or original Core Note".into(),
        );
    }
    let queued_after_startup = startup.queue.load(Ordering::SeqCst);
    let mut check = Checks {
        store,
        service,
        key,
        note: &startup.note,
        document: &startup.document,
        state: startup.state,
        steps: vec![],
    };
    check
        .apply(
            "relation kind",
            NalMutation::SetConceptKind {
                concept_id: ENDPOINT.into(),
                concept_kind: "relation".into(),
            },
            1,
        )
        .await?;
    if !check
        .state
        .concepts
        .iter()
        .any(|c| c.id == ENDPOINT && c.concept_kind == "relation")
    {
        return Err("SetConceptKind did not preserve the original relation role".into());
    }
    for enabled in [true, true, false, false] {
        check
            .apply(
                "transitivity including no-ops",
                NalMutation::SetTransitivity {
                    relation_concept_id: ENDPOINT.into(),
                    enabled,
                },
                1,
            )
            .await?;
        let matching = check
            .state
            .rules
            .iter()
            .filter(|r| r.relation_concept_id == ENDPOINT)
            .collect::<Vec<_>>();
        if (enabled
            && (matching.len() != 1
                || matching[0].kind != "transitive"
                || matching[0].id != "transitive:bootstrap-endpoint"))
            || (!enabled && !matching.is_empty())
        {
            return Err("transitivity rule identity or no-op semantics changed".into());
        }
    }
    let original = statement(
        "mutation-original-expression",
        term(&survivor),
        term(&survivor),
    );
    check
        .apply(
            "add expression",
            NalMutation::AddStatements {
                statements: vec![original.clone()],
            },
            1,
        )
        .await?;
    let statements_before = check.state.statements.clone();
    check
        .apply(
            "duplicate expression keeps original id",
            NalMutation::AddStatements {
                statements: vec![statement(
                    "mutation-ignored-caller-id",
                    term(&survivor),
                    term(&survivor),
                )],
            },
            1,
        )
        .await?;
    if check.state.statements != statements_before || !check.state.statements.contains(&original) {
        return Err("duplicate expression replaced the original statement identity".into());
    }
    let statement_link = annotation(
        "mutation-statement-note",
        "statement",
        &original.id,
        &startup.note,
    );
    check
        .apply(
            "link statement",
            NalMutation::LinkAnnotation {
                annotation: statement_link.clone(),
            },
            1,
        )
        .await?;
    if !check.state.annotations.contains(&statement_link) {
        return Err("statement annotation missing".into());
    }
    check
        .apply(
            "delete statement cascades annotation",
            NalMutation::DeleteStatement {
                id: original.id.clone(),
            },
            1,
        )
        .await?;
    if check.state.statements.iter().any(|s| s.id == original.id)
        || check
            .state
            .annotations
            .iter()
            .any(|a| a.id == statement_link.id)
    {
        return Err("DeleteStatement did not cascade the original annotation".into());
    }
    let retained = annotation(RETAINED_ANNOTATION, "concept", &survivor, &startup.note);
    check
        .apply(
            "link concept",
            NalMutation::LinkAnnotation {
                annotation: retained.clone(),
            },
            1,
        )
        .await?;
    let linked = check.state.annotations.clone();
    check
        .apply(
            "exact repeated link",
            NalMutation::LinkAnnotation {
                annotation: retained.clone(),
            },
            1,
        )
        .await?;
    if check.state.annotations != linked {
        return Err("repeated link changed original annotation identity".into());
    }
    for _ in 0..2 {
        check
            .apply(
                "unlink including absent no-op",
                NalMutation::UnlinkAnnotation {
                    id: retained.id.clone(),
                },
                1,
            )
            .await?;
        if check.state.annotations.iter().any(|a| a.id == retained.id) {
            return Err("UnlinkAnnotation retained its target".into());
        }
    }
    check
        .apply(
            "retain final annotation",
            NalMutation::LinkAnnotation {
                annotation: retained.clone(),
            },
            1,
        )
        .await?;
    let nested = statement(
        "mutation-nested-reference",
        product(vec![
            term(&survivor),
            product(vec![term(ENDPOINT), term(&survivor)]),
        ]),
        term(&survivor),
    );
    let relation = statement(
        "mutation-endpoint-relation",
        product(vec![term(&survivor), term(ENDPOINT)]),
        term(ENDPOINT),
    );
    check
        .apply(
            "nested concept references",
            NalMutation::AddStatements {
                statements: vec![nested.clone(), relation.clone()],
            },
            1,
        )
        .await?;
    check
        .apply(
            "rule for deleted concept",
            NalMutation::SetTransitivity {
                relation_concept_id: ENDPOINT.into(),
                enabled: true,
            },
            1,
        )
        .await?;
    let deleted_link = annotation(
        "mutation-deleted-concept-note",
        "concept",
        ENDPOINT,
        &startup.note,
    );
    check
        .apply(
            "annotation for deleted concept",
            NalMutation::LinkAnnotation {
                annotation: deleted_link.clone(),
            },
            1,
        )
        .await?;
    // The original concept/term-link triggers already create this FTS row.
    // Check its complete content against the actual synthetic source, then
    // require deletion below; do not insert a second search row.
    if readback::scalar(
        store,
        "select count(*) from concept_search where concept_id='bootstrap-endpoint'",
    )? != 1
    {
        return Err("synthetic endpoint must have one original FTS row".into());
    }
    let matching: i64 = store.connection()?.query_row(
        "select count(*) from concept_search s join concepts c on c.id=s.concept_id
         join concept_terms ct on ct.concept_id=c.id and ct.relation_kind='preferred'
         join terms t on t.id=ct.term_id
         where c.id=?1 and c.deleted_at is null and s.terms=t.text
         and t.id='bootstrap-endpoint-term' and t.text='Synthetic original endpoint'
         and s.label=c.label and c.label='Synthetic original endpoint'
         and s.explanation=c.explanation and c.explanation='Synthetic original explanation'
         and s.qualifiers=trim(c.disambiguation||' '||c.domain||' '||c.tradition||' '||c.period)",
        [ENDPOINT],
        |row| row.get(0),
    )?;
    if matching != 1 {
        return Err("original FTS row differs from the actual synthetic endpoint/term".into());
    }
    let before_delete = readback::snapshot(store)?;
    check
        .apply(
            "soft delete cascades references",
            NalMutation::DeleteConcept {
                id: ENDPOINT.into(),
            },
            2,
        )
        .await?;
    let after_delete = readback::snapshot(store)?;
    if check.state.concepts.iter().any(|c| c.id == ENDPOINT)
        || check.state.statements.iter().any(|s| {
            [nested.id.as_str(), relation.id.as_str(), fixture::REPAIR].contains(&s.id.as_str())
        })
        || check
            .state
            .annotations
            .iter()
            .any(|a| a.id == deleted_link.id || a.id == "bootstrap-repair-note")
        || check
            .state
            .rules
            .iter()
            .any(|r| r.relation_concept_id == ENDPOINT)
        || readback::scalar(
            store,
            "select deleted_at is not null from concepts where id='bootstrap-endpoint'",
        )? != 1
        || readback::scalar(
            store,
            "select count(*) from concept_search where concept_id='bootstrap-endpoint'",
        )? != 0
        || readback::scalar(
            store,
            "select kind='relation' from nal_concept_kinds where concept_id='bootstrap-endpoint'",
        )? != 1
    {
        return Err("concept deletion did not retain physical role/history while cascading nested references and FTS".into());
    }
    for table in [
        "terms",
        "concept_terms",
        "research_object_revisions",
        "semantic_representations",
        "embedding_spaces",
        "embedding_index_generations",
        "embedding_records",
        "embedding_outbox_jobs",
        "note_pages",
        "note_blocks",
        "note_revisions",
    ] {
        if before_delete[table] != after_delete[table] {
            return Err(format!(
                "concept deletion changed retained synthetic metadata table {table}"
            )
            .into());
        }
    }
    let protected = check
        .state
        .statements
        .iter()
        .filter(|s| {
            contains(&s.subject, fixture::PLACEHOLDER)
                || contains(&s.predicate, fixture::PLACEHOLDER)
        })
        .collect::<Vec<_>>();
    if protected.len() != 1 {
        return Err("expected one original placeholder protection statement".into());
    }
    let protection_id = protected[0].id.clone();
    check
        .apply(
            "remove final protection without post-mutation cleanup",
            NalMutation::DeleteStatement { id: protection_id },
            1,
        )
        .await?;
    if !check
        .state
        .concepts
        .iter()
        .any(|c| c.id == fixture::PLACEHOLDER)
    {
        return Err("mutation incorrectly ran cleanup after removing the last protection".into());
    }
    let before_cleanup = check.state.clone();
    let final_state = read(store, service, key).await?;
    if final_state.revision != before_cleanup.revision + 1
        || final_state
            .concepts
            .iter()
            .any(|c| c.id == fixture::PLACEHOLDER)
        || final_state.annotations != vec![retained.clone()]
        || fixture::note(store, &startup.note)? != startup.document
        || readback::metadata(store)? != startup.metadata
        || startup.queue.load(Ordering::SeqCst) != queued_after_startup
    {
        return Err(
            "explicit next read cleanup changed original Note/metadata/queue/annotation semantics"
                .into(),
        );
    }
    let snapshot = readback::snapshot(store)?;
    if read(store, service, key).await? != final_state || readback::snapshot(store)? != snapshot {
        return Err("completed final read wrote or changed the frozen result".into());
    }
    Ok(
        json!({"state":final_state,"sqlSnapshot":snapshot,"metadata":startup.metadata,
        "noteId":startup.note,"noteDocument":startup.document,"annotation":retained,
        "steps":check.steps,"failures":fault_results,"allSevenOriginalVariants":true,
        "sameCoreNoteThroughout":true,"noPostMutationCleanup":true,"explicitReadCleaned":true,
        "wholeRollbackSnapshotsCompared":true,"actualMetadataCompared":true,
        "startupQueueInsertAttemptsPerCall":8,"noFurtherQueueInsertAttempts":true,
        "coreOnly":true}),
    )
}

fn contains(term: &NalTerm, target: &str) -> bool {
    match term {
        NalTerm::Concept { id } => id == target,
        NalTerm::Product { terms } => terms.iter().any(|term| contains(term, target)),
        NalTerm::Paper { .. } => false,
    }
}

pub(super) async fn reopen(
    store: &AppStore,
    service: &Arc<LiteratureTraceService>,
    key: &str,
    directory: &Path,
) -> Result<Value, Error> {
    cold::run(store, service, key, directory).await
}
