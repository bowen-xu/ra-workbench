use super::{
    ReadingUnitFailure as F, ReadingUnitOutcome as O, ReadingUnitProblem as E,
    ReadingUnitResources, ReadingUnitResult,
    data::{self, optional, text},
};
use crate::data_interface::{
    DatabaseGuard, guarded_transaction,
    plugin_resources::{ColumnCodec as C, RowPolicy as P},
};
use research_workbench_native_sdk::{
    data::DataValue as V,
    reading_units::{
        ReadingUnitPlanV1, ReadingUnitPreparationV1, ReadingUnitRequestV1, ReadingUnitRulesV1,
        StructuredReadingUnit, StructuredReadingUnitDraft,
    },
};
use rusqlite::{Connection, DropBehavior, Transaction, TransactionBehavior};
use uuid::Uuid;

pub(crate) fn check_preparation(
    input: &ReadingUnitRequestV1,
    outcome: &ReadingUnitPreparationV1,
) -> Result<(), F> {
    let valid = match (input, outcome) {
        (
            ReadingUnitRequestV1::List {
                project_id,
                paper_id,
            },
            ReadingUnitPreparationV1::Ready { plan },
        ) => {
            matches!(plan.as_ref(),ReadingUnitPlanV1::List{project_id:a,paper_id:b} if project_id==a && paper_id==b)
        }
        (ReadingUnitRequestV1::Save { draft }, ReadingUnitPreparationV1::Ready { plan }) => {
            matches!(plan.as_ref(),ReadingUnitPlanV1::Save{draft:original,..} if draft==original)
        }
        (
            _,
            ReadingUnitPreparationV1::Rejected {
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
    r: &ReadingUnitResources,
    outcome: ReadingUnitPreparationV1,
) -> Result<ReadingUnitResult, F> {
    match outcome {
        ReadingUnitPreparationV1::Rejected { input, reason } => {
            let mut f = F::new("validation", E::Domain(reason));
            if let ReadingUnitRequestV1::Save { draft } = *input {
                f.reading_unit_id = draft.id;
            }
            Err(f)
        }
        ReadingUnitPreparationV1::Ready { plan } => match *plan {
            ReadingUnitPlanV1::List {
                project_id,
                paper_id,
            } => {
                let ids = r
                    .read(
                        guard,
                        "list",
                        vec![
                            optional(project_id.as_deref()),
                            optional(paper_id.as_deref()),
                        ],
                        P::All,
                        &[C::String],
                    )
                    .map_err(|e| F::data("list", e))?
                    .rows
                    .into_iter()
                    .map(|row| data::Row::new(row).text())
                    .collect::<Result<Vec<_>, _>>()?;
                ids.iter()
                    .map(|id| data::unit(guard, r, id))
                    .collect::<Result<Vec<_>, _>>()
                    .map(ReadingUnitResult::Units)
            }
            ReadingUnitPlanV1::Save {
                draft,
                title,
                body,
                evidence_stance,
                evidence_rationale,
                rules,
            } => guarded_transaction::run(guard, |db| {
                let result = save(
                    db,
                    r,
                    &draft,
                    &title,
                    &body,
                    &evidence_stance,
                    &evidence_rationale,
                    &rules,
                )
                .map(|v| ReadingUnitResult::Unit(Box::new(v)))
                .map_err(|mut f| {
                    if f.reading_unit_id.is_none() {
                        f.reading_unit_id = draft.id.clone();
                    }
                    f
                });
                let unknown = result.as_ref().is_err_and(|f| f.outcome == O::Unknown);
                (result, unknown)
            }),
        },
    }
}
#[allow(clippy::too_many_arguments)] // Original fixed plan fields and bindings.
fn save(
    db: &Connection,
    r: &ReadingUnitResources,
    draft: &StructuredReadingUnitDraft,
    title: &str,
    body: &str,
    stance: &str,
    rationale: &str,
    rules: &ReadingUnitRulesV1,
) -> Result<StructuredReadingUnit, F> {
    let was_autocommit = db.is_autocommit();
    let mut tx = Transaction::new_unchecked(db, TransactionBehavior::Deferred).map_err(|e| {
        let mut f = F::sql("begin", &e);
        if was_autocommit && !db.is_autocommit() {
            f.outcome = O::Unknown;
        }
        f
    })?;
    tx.set_drop_behavior(DropBehavior::Ignore);
    let id = match mutate(&tx, r, draft, title, body, stance, rationale, rules) {
        Ok(id) => id,
        Err(mut f) => {
            settle(db, tx.rollback(), &mut f);
            return Err(f);
        }
    };
    if let Err(e) = tx.commit() {
        let mut f = F::sql("commit", &e);
        f.reading_unit_id = Some(id);
        if db.is_autocommit() {
            f.outcome = O::Unknown;
        } else {
            settle(db, db.execute_batch("ROLLBACK"), &mut f);
        }
        return Err(f);
    }
    if !db.is_autocommit() {
        let mut f = F::new("commit", E::Owner("READING_UNIT_TRANSACTION_UNSETTLED"));
        f.outcome = O::Unknown;
        f.reading_unit_id = Some(id);
        return Err(f);
    }
    // Original save has NO full DTO read inside T1. Both queries occur only now.
    data::unit(db, r, &id).map_err(|mut f| {
        f.phase = "postcommit-read";
        f.outcome = O::Committed;
        f.reading_unit_id = Some(id);
        f
    })
}
#[allow(clippy::too_many_arguments)] // Preserve original mutation/revision/evidence ordering.
fn mutate(
    tx: &Transaction<'_>,
    r: &ReadingUnitResources,
    draft: &StructuredReadingUnitDraft,
    title: &str,
    body: &str,
    stance: &str,
    rationale: &str,
    rules: &ReadingUnitRulesV1,
) -> Result<String, F> {
    let (id, revision) = if let Some(id) = &draft.id {
        let current =
            data::first(tx, r, "current-revision", vec![text(id)], &[C::I64])?.integer()?;
        if draft.expected_revision != Some(current) {
            let mut f = F::new("current-revision", E::RevisionConflict);
            f.current_revision = Some(current);
            return Err(f);
        }
        let next = current + 1;
        r.write(
            tx,
            "update",
            vec![
                text(id),
                optional(draft.project_id.as_deref()),
                optional(draft.paper_id.as_deref()),
                text(&draft.unit_kind),
                text(title),
                text(body),
                text(&draft.epistemic_role),
                V::Integer(next),
            ],
        )
        .map_err(|e| F::data("update", e))?;
        (id.clone(), next)
    } else {
        if let Some(reason) = rules.new_expected_rejection {
            return Err(F::new("current-revision", E::Domain(reason)));
        }
        let id = Uuid::new_v4().to_string();
        r.write(
            tx,
            "insert",
            vec![
                text(&id),
                optional(draft.project_id.as_deref()),
                optional(draft.paper_id.as_deref()),
                text(&draft.unit_kind),
                text(title),
                text(body),
                text(&draft.epistemic_role),
            ],
        )
        .map_err(|e| {
            let mut f = F::data("insert", e);
            f.reading_unit_id = Some(id.clone());
            f
        })?;
        (id, rules.initial_revision)
    };
    (||{
        // Preserve the original manual history shape, even when an existing
        // row retains its AI provenance. Do not replace this with the DTO.
        let snapshot=serde_json::json!({"id":id,"projectId":draft.project_id,"paperId":draft.paper_id,"unitKind":draft.unit_kind,"title":title,"body":body,"provenanceKind":rules.provenance_kind,"epistemicRole":draft.epistemic_role,"revision":revision});
        let revision_id=Uuid::new_v4().to_string();
        let json=serde_json::to_string(&snapshot).map_err(|_|F::new("revision",E::Codec))?;
        r.write(tx,"revision",vec![V::Text(revision_id),text(&rules.object_kind),text(&id),V::Integer(revision),V::Text(json),text(&rules.editor_kind),text(&rules.change_reason)]).map_err(|e|F::data("revision",e))?;
        if let Some(anchor)=&draft.anchor_id{
            let evidence_id=Uuid::new_v4().to_string();
            r.write(tx,"evidence-upsert",vec![V::Text(evidence_id),text(&id),text(anchor),text(stance),text(rationale)]).map_err(|e|F::data("evidence-upsert",e))?;
        }
        Ok(())
    })().map_err(|mut f:F|{f.reading_unit_id=Some(id.clone());f})?;
    Ok(id)
}
fn settle(db: &Connection, result: rusqlite::Result<()>, failure: &mut F) {
    match result {
        Ok(()) if db.is_autocommit() => failure.outcome = O::RolledBack,
        result => {
            failure.outcome = O::Unknown;
            failure.rollback_error = Some(Box::new(match result {
                Err(e) => crate::runtime_plugins::native::SafeDatabaseError::sql("rollback", &e),
                Ok(()) => crate::runtime_plugins::native::SafeDatabaseError::invariant(
                    "reading_unit_transaction_unsettled",
                    "rollback",
                ),
            }));
        }
    }
}
