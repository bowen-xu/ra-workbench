use super::{
    ConceptFailure as F, ConceptOutcome as O, ConceptProblem as E, ConceptResources, ConceptResult,
    data::{self, optional, text},
};
use crate::data_interface::{
    DatabaseGuard, guarded_transaction,
    plugin_resources::{ColumnCodec as C, RowPolicy as P},
};
use research_workbench_native_sdk::{
    concepts::{
        Concept, ConceptPlanV1, ConceptPreparationV1, ConceptRejectionV1 as R, ConceptRequestV1,
        ConceptRulesV1, ConceptTermPlanV1, NewConceptDetails,
    },
    data::DataValue as V,
};
use rusqlite::{Connection, DropBehavior, Transaction, TransactionBehavior};
use uuid::Uuid;

pub(crate) fn check_preparation(
    input: &ConceptRequestV1,
    outcome: &ConceptPreparationV1,
) -> Result<(), F> {
    let valid = match (input, outcome) {
        (ConceptRequestV1::Search { query }, ConceptPreparationV1::Ready { plan }) => {
            matches!(plan.as_ref(),ConceptPlanV1::Search{query:original,..} if query==original)
        }
        (ConceptRequestV1::Save { draft }, ConceptPreparationV1::Ready { plan }) => {
            matches!(plan.as_ref(),ConceptPlanV1::Save{draft:original,..} if draft==original)
        }
        (
            _,
            ConceptPreparationV1::Rejected {
                input: original, ..
            },
        ) => input == original.as_ref(),
    };
    if valid {
        Ok(())
    } else {
        Err(F::new("preparation", E::Binding))
    }
}

pub(crate) fn apply(
    guard: &mut DatabaseGuard<'_>,
    r: &ConceptResources,
    outcome: ConceptPreparationV1,
) -> Result<ConceptResult, F> {
    match outcome {
        ConceptPreparationV1::Rejected { reason, .. } => {
            Err(F::new("validation", E::Domain(reason)))
        }
        ConceptPreparationV1::Ready { plan } => match *plan {
            ConceptPlanV1::Search {
                normalized,
                pattern,
                ..
            } => {
                let rows = r
                    .read(
                        guard,
                        "search",
                        vec![V::Text(normalized), V::Text(pattern)],
                        P::All,
                        &[C::String],
                    )
                    .map_err(|e| F::data("search", e))?;
                let ids = rows
                    .rows
                    .into_iter()
                    .map(|row| data::Row::new(row).text())
                    .collect::<Result<Vec<_>, _>>()?;
                ids.iter()
                    .map(|id| data::concept(guard, r, id))
                    .collect::<Result<Vec<_>, _>>()
                    .map(ConceptResult::Concepts)
            }
            ConceptPlanV1::Save {
                draft,
                details,
                term,
                rules,
            } => guarded_transaction::run(guard, |db| {
                let result = save(db, r, &draft, &details, &term, &rules)
                    .map(|value| ConceptResult::Concept(Box::new(value)))
                    .map_err(|mut failure| {
                        if failure.concept_id.is_none() {
                            failure.concept_id = draft.id.clone();
                        }
                        failure
                    });
                let unknown = result.as_ref().is_err_and(|e| e.outcome == O::Unknown);
                (result, unknown)
            }),
        },
    }
}

fn save(
    db: &Connection,
    r: &ConceptResources,
    draft: &research_workbench_native_sdk::concepts::ConceptDraft,
    details: &NewConceptDetails,
    term: &ConceptTermPlanV1,
    rules: &ConceptRulesV1,
) -> Result<Concept, F> {
    let was_autocommit = db.is_autocommit();
    let mut tx = Transaction::new_unchecked(db, TransactionBehavior::Deferred).map_err(|e| {
        let mut f = F::sql("begin", &e);
        if was_autocommit && !db.is_autocommit() {
            f.outcome = O::Unknown;
        }
        f
    })?;
    tx.set_drop_behavior(DropBehavior::Ignore);
    let id = match mutate(&tx, r, draft, details, term, rules) {
        Ok(id) => id,
        Err(mut e) => {
            settle(db, tx.rollback(), &mut e);
            return Err(e);
        }
    };
    if let Err(cause) = tx.commit() {
        let mut e = F::sql("commit", &cause);
        e.concept_id = Some(id);
        if db.is_autocommit() {
            e.outcome = O::Unknown;
        } else {
            settle(db, db.execute_batch("ROLLBACK"), &mut e);
        }
        return Err(e);
    }
    if !db.is_autocommit() {
        let mut f = F::new("commit", E::Owner("CONCEPT_TRANSACTION_UNSETTLED"));
        f.outcome = O::Unknown;
        f.concept_id = Some(id);
        return Err(f);
    }
    data::concept(db, r, &id).map_err(|mut e| {
        e.phase = "postcommit-read";
        e.outcome = O::Committed;
        e.concept_id = Some(id);
        e
    })
}

fn mutate(
    tx: &Transaction<'_>,
    r: &ConceptResources,
    draft: &research_workbench_native_sdk::concepts::ConceptDraft,
    details: &NewConceptDetails,
    term_plan: &ConceptTermPlanV1,
    rules: &ConceptRulesV1,
) -> Result<String, F> {
    name_available(tx, r, &details.label, draft.id.as_deref())?;
    let term = ensure_term(tx, r, term_plan, rules)?;
    let id = if let Some(id) = &draft.id {
        if boolean(tx, r, "merged", vec![text(id)])? {
            return Err(F::new("merged", E::Domain(R::MergedIdentity)));
        }
        let revision =
            data::first(tx, r, "current-revision", vec![text(id)], &[C::I64])?.integer()?;
        if draft.expected_revision != Some(revision) {
            let mut f = F::new("current-revision", E::RevisionConflict);
            f.current_revision = Some(revision);
            f.concept_id = Some(id.clone());
            return Err(f);
        }
        write(tx, r, "alias-update", vec![text(id), text(&term.id)])?;
        write(tx, r, "preferred-upsert", vec![text(id), text(&term.id)])?;
        let mut args = vec![text(id)];
        args.extend(fields(details));
        args.push(V::Integer(revision + 1));
        write(tx, r, "update", args)?;
        let actual = data::concept(tx, r, id)?;
        revision_in(
            tx,
            r,
            &rules.concept_object_kind,
            id,
            actual.revision,
            &actual,
            &rules.editor_kind,
            &rules.concept_reason,
            "revision",
        )?;
        semantic(tx, r, &actual)?;
        id.clone()
    } else {
        if let Some(reason) = rules.new_expected_rejection {
            return Err(F::new("current-revision", E::Domain(reason)));
        }
        // insert_concept originally repeats availability after ensure_term;
        // trigger-created collisions must still be observed at this position.
        name_available(tx, r, &details.label, None)?;
        let id = Uuid::new_v4().to_string();
        (|| {
            let mut args = vec![text(&id)];
            args.extend(fields(details));
            args.push(text(&rules.provenance_kind));
            write(tx, r, "concept-insert", args)?;
            write(
                tx,
                r,
                "concept-preferred-insert",
                vec![text(&id), text(&term.id)],
            )?;
            let actual = data::concept(tx, r, &id)?;
            revision_in(
                tx,
                r,
                &rules.concept_object_kind,
                &id,
                rules.initial_revision,
                &actual,
                &rules.editor_kind,
                &rules.concept_reason,
                "revision",
            )?;
            semantic(tx, r, &actual)
        })()
        .map_err(|mut failure| {
            failure.concept_id = Some(id.clone());
            failure
        })?;
        id
    };
    Ok(id)
}

fn fields(d: &NewConceptDetails) -> Vec<V> {
    vec![
        text(&d.label),
        text(&d.explanation),
        text(&d.disambiguation),
        text(&d.domain),
        text(&d.tradition),
        text(&d.period),
    ]
}
fn write(
    tx: &Transaction<'_>,
    r: &ConceptResources,
    key: &'static str,
    args: Vec<V>,
) -> Result<u64, F> {
    r.write(tx, key, args).map_err(|e| F::data(key, e))
}
fn boolean(
    db: &Connection,
    r: &ConceptResources,
    key: &'static str,
    args: Vec<V>,
) -> Result<bool, F> {
    data::first(db, r, key, args, &[C::Bool])?
        .integer()
        .map(|v| v != 0)
}
fn name_available(
    db: &Connection,
    r: &ConceptResources,
    label: &str,
    id: Option<&str>,
) -> Result<(), F> {
    if let Some(id) = id
        && !boolean(db, r, "name-active", vec![text(id)])?
    {
        return Err(F::new("name-active", E::Missing));
    }
    if boolean(
        db,
        r,
        "concept-name-available",
        vec![text(label), optional(id)],
    )? {
        return Err(F::new(
            "concept-name-available",
            E::Domain(R::NameUnavailable),
        ));
    }
    Ok(())
}
fn ensure_term(
    tx: &Transaction<'_>,
    r: &ConceptResources,
    term: &ConceptTermPlanV1,
    rules: &ConceptRulesV1,
) -> Result<research_workbench_native_sdk::concepts::Term, F> {
    if let Some(reason) = term.rejection {
        return Err(F::new("term-validation", E::Domain(reason)));
    }
    let rows = r
        .read(
            tx,
            "term-lookup",
            vec![text(&term.normalized_text), text(&term.language)],
            P::FirstOptional,
            &[C::String],
        )
        .map_err(|e| F::data("term-lookup", e))?;
    if let Some(row) = rows.rows.into_iter().next() {
        return data::term(tx, r, &data::Row::new(row).text()?);
    }
    let id = Uuid::new_v4().to_string();
    write(
        tx,
        r,
        "term-insert",
        vec![
            text(&id),
            text(&term.text),
            text(&term.normalized_text),
            text(&term.language),
            text(&rules.provenance_kind),
        ],
    )?;
    let actual = data::term(tx, r, &id)?;
    revision_in(
        tx,
        r,
        &rules.term_object_kind,
        &id,
        rules.initial_revision,
        &actual,
        &rules.editor_kind,
        &rules.term_reason,
        "term-revision",
    )?;
    Ok(actual)
}
#[allow(clippy::too_many_arguments)] // Original fixed revision bindings and failure position.
fn revision_in(
    tx: &Transaction<'_>,
    r: &ConceptResources,
    kind: &str,
    id: &str,
    revision: i64,
    actual: &impl serde::Serialize,
    editor: &str,
    reason: &str,
    phase: &'static str,
) -> Result<(), F> {
    let value = serde_json::to_value(actual).map_err(|_| F::new(phase, E::Codec))?;
    let revision_id = Uuid::new_v4().to_string();
    let json = serde_json::to_string(&value).map_err(|_| F::new(phase, E::Codec))?;
    r.write(
        tx,
        "research-revision-insert",
        vec![
            V::Text(revision_id),
            text(kind),
            text(id),
            V::Integer(revision),
            V::Text(json),
            text(editor),
            text(reason),
        ],
    )
    .map_err(|e| F::data(phase, e))?;
    Ok(())
}
pub(super) fn semantic(
    tx: &Transaction<'_>,
    r: &ConceptResources,
    actual: &Concept,
) -> Result<(), F> {
    super::semantic::register(
        tx,
        super::semantic::SemanticResources::from_concepts(r),
        actual,
    )
    .map_err(|error| {
        let mut f = match error.cause {
            crate::runtime_plugins::literature_nal_bootstrap::CaptureError::Data(cause) => {
                F::data("semantic", cause)
            }
            crate::runtime_plugins::literature_nal_bootstrap::CaptureError::Owner(cause) => {
                F::new("semantic", E::Database(cause))
            }
            _ => F::new("semantic", E::Codec),
        };
        f.semantic_phase = Some(error.phase);
        f
    })
}
fn settle(db: &Connection, result: rusqlite::Result<()>, failure: &mut F) {
    match result {
        Ok(()) if db.is_autocommit() => failure.outcome = O::RolledBack,
        result => {
            failure.outcome = O::Unknown;
            failure.rollback_error = Some(Box::new(match result {
                Err(cause) => {
                    crate::runtime_plugins::native::SafeDatabaseError::sql("rollback", &cause)
                }
                Ok(()) => crate::runtime_plugins::native::SafeDatabaseError::invariant(
                    "concept_transaction_unsettled",
                    "rollback",
                ),
            }));
        }
    }
}
