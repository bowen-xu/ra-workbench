use super::*;
use crate::data_interface::plugin_resources::RowPolicy;

pub(super) fn update(
    db: &Connection,
    resources: &MetadataResources,
    catalog: &CatalogResources,
    plan: &MetadataUpdatePlanV1,
) -> Result<MetadataExecution> {
    let id = &plan.input.item_id;
    let result = (|| {
        current(db, catalog, id, MetadataPhaseOutcome::NotStarted)?;
        transaction::run(db, |tx| {
            let rows = resources
                .read(
                    tx,
                    "provenance-confirmed",
                    vec![text(id), text(&plan.field_name)],
                    RowPolicy::FirstOptional,
                    &[ColumnCodec::Bool],
                )
                .map_err(|error| ("provenance-confirmed", MetadataProblem::Data(error)))?;
            let unconfirmed = match rows.rows.as_slice() {
                [] => false,
                [row] => match row.as_slice() {
                    [DataValue::Integer(value)] => *value == 0,
                    _ => return Err(("provenance-confirmed", MetadataProblem::Binding)),
                },
                _ => return Err(("provenance-confirmed", MetadataProblem::Binding)),
            };
            patch::apply(tx, resources, id, &plan.patch)?;
            write(tx, resources, "item-timestamp", vec![text(id)])?;
            let provenance = if unconfirmed {
                &plan.sources.unconfirmed
            } else {
                &plan.sources.otherwise
            };
            derivation::provenance(tx, resources, id, &plan.field_name, provenance)?;
            derivation::refresh(tx, resources, id, &plan.derivation)
        })?;
        current(db, catalog, id, MetadataPhaseOutcome::Committed).map(MetadataExecution::Item)
    })();
    item_failure(result, id)
}

pub(super) fn confirm(
    db: &Connection,
    resources: &MetadataResources,
    catalog: &CatalogResources,
    outcome: &MetadataConfirmOutcomeV1,
) -> Result<MetadataExecution> {
    let id = match outcome {
        MetadataConfirmOutcomeV1::Ready { input, .. }
        | MetadataConfirmOutcomeV1::Rejected { input, .. } => &input.item_id,
    };
    let result = (|| {
        current(db, catalog, id, MetadataPhaseOutcome::NotStarted)?;
        let fields = match outcome {
            MetadataConfirmOutcomeV1::Ready { fields, .. } => fields,
            MetadataConfirmOutcomeV1::Rejected { error, .. } => {
                return Err(MetadataFailure::new(
                    "confirmation-validation",
                    MetadataProblem::Domain(error.clone()),
                    MetadataPhaseOutcome::NotStarted,
                ));
            }
        };
        transaction::run(db, |tx| {
            for field in fields {
                if write(
                    tx,
                    resources,
                    "confirm-field",
                    vec![text(id), text(&field.field_name)],
                )? == 0
                {
                    return Err((
                        "confirm-field",
                        MetadataProblem::Domain(field.missing_provenance.clone()),
                    ));
                }
            }
            Ok(())
        })?;
        current(db, catalog, id, MetadataPhaseOutcome::Committed).map(MetadataExecution::Item)
    })();
    item_failure(result, id)
}

pub(super) fn fields(
    db: &Connection,
    resources: &MetadataResources,
    catalog: &CatalogResources,
    outcome: &MetadataFieldsApplyOutcomeV1,
) -> Result<MetadataExecution> {
    let (input, decisions, derivation) = match outcome {
        MetadataFieldsApplyOutcomeV1::Ready {
            input,
            decisions,
            derivation,
        } => (input, decisions, derivation),
        MetadataFieldsApplyOutcomeV1::Rejected { input, error } => {
            let mut failure = MetadataFailure::new(
                "fields-validation",
                MetadataProblem::Domain(error.clone()),
                MetadataPhaseOutcome::NotStarted,
            );
            failure.item_id = input.header.item_id.clone();
            return Err(failure);
        }
    };
    let id = input.header.item_id.as_deref().ok_or_else(|| {
        MetadataFailure::new(
            "fields-input",
            MetadataProblem::Binding,
            MetadataPhaseOutcome::NotStarted,
        )
    })?;
    let result = (|| {
        transaction::run(db, |tx| {
            for decision in decisions {
                match decision {
                    MetadataPreparedDecisionV1::Candidate {
                        field_name,
                        patch,
                        provenance,
                    } => {
                        patch::apply(tx, resources, id, patch)?;
                        derivation::provenance(tx, resources, id, field_name, provenance)?;
                    }
                    MetadataPreparedDecisionV1::Current {
                        field_name,
                        on_missing,
                    } => {
                        if write(
                            tx,
                            resources,
                            "confirm-current-field",
                            vec![text(id), text(field_name)],
                        )? == 0
                        {
                            derivation::provenance(tx, resources, id, field_name, on_missing)?;
                        }
                    }
                }
            }
            write(tx, resources, "item-timestamp", vec![text(id)])?;
            derivation::refresh(tx, resources, id, derivation)?;
            finish(
                tx,
                resources,
                &input.candidate_id,
                id,
                input.header.import_result_id.as_deref(),
            )
        })?;
        current(db, catalog, id, MetadataPhaseOutcome::Committed).map(MetadataExecution::Item)
    })();
    item_failure(result, id)
}

pub(super) fn full_second(
    db: &Connection,
    resources: &MetadataResources,
    catalog: &CatalogResources,
    plan: &MetadataFullApplyPlanV1,
    id: &str,
) -> Result<MetadataExecution> {
    transaction::run(db, |tx| {
        for source in &plan.field_sources {
            write(
                tx,
                resources,
                "candidate-provenance-upsert",
                vec![
                    text(id),
                    text(&source.field_name),
                    text(&source.provenance.source_kind),
                    optional_text(source.provenance.source_reference.as_deref()),
                ],
            )?;
        }
        finish(
            tx,
            resources,
            &plan.input.candidate_id,
            id,
            plan.input.header.import_result_id.as_deref(),
        )
    })?;
    current(db, catalog, id, MetadataPhaseOutcome::Committed).map(MetadataExecution::Item)
}

fn finish(
    tx: &Transaction<'_>,
    resources: &MetadataResources,
    candidate_id: &str,
    item_id: &str,
    result_id: Option<&str>,
) -> StepResult<()> {
    write(tx, resources, "candidate-applied", vec![text(candidate_id)])?;
    if let Some(result_id) = result_id {
        write(
            tx,
            resources,
            "result-imported",
            vec![text(result_id), text(item_id)],
        )?;
        write(tx, resources, "job-completed", vec![text(result_id)])?;
    }
    Ok(())
}

pub(super) fn reject(
    db: &Connection,
    resources: &MetadataResources,
    plan: &MetadataRejectPlanV1,
) -> Result<MetadataExecution> {
    // The caller has just re-read the original optional raw SELECT under this
    // same guard. Do not add a third read, parse draft JSON or open a transaction.
    if !db.is_autocommit() {
        return Err(MetadataFailure::new(
            "admission",
            MetadataProblem::Owner("connection_unsettled"),
            MetadataPhaseOutcome::Unknown,
        ));
    }
    let mut completed = Vec::new();
    let affected = statement(
        db,
        resources,
        "candidate-rejected",
        vec![text(&plan.input.candidate_id)],
        &mut completed,
    )?;
    if affected == 0 {
        let mut error = MetadataFailure::new(
            "candidate-rejected",
            MetadataProblem::Domain(plan.zero_affected.clone()),
            MetadataPhaseOutcome::StatementReturned {
                affected_rows: affected,
            },
        );
        error.completed_statements = completed;
        return Err(error);
    }
    if let Some(result_id) = &plan.input.import_result_id {
        statement(
            db,
            resources,
            "result-cancelled",
            vec![text(result_id)],
            &mut completed,
        )?;
        statement(
            db,
            resources,
            "job-cancelled",
            vec![text(result_id)],
            &mut completed,
        )?;
    }
    Ok(MetadataExecution::Done)
}

fn statement(
    db: &Connection,
    resources: &MetadataResources,
    key: &'static str,
    arguments: Vec<DataValue>,
    completed: &mut Vec<&'static str>,
) -> Result<u64> {
    let result = resources.write_on_connection(db, key, arguments);
    let settled = db.is_autocommit();
    let result = match result {
        Err(error) => {
            let outcome = if !settled {
                MetadataPhaseOutcome::Unknown
            } else if matches!(error.phase, "validate" | "prepare" | "parameters") {
                MetadataPhaseOutcome::NotStarted
            } else {
                MetadataPhaseOutcome::StatementFailed
            };
            Err(MetadataFailure::new(
                key,
                MetadataProblem::Data(error),
                outcome,
            ))
        }
        Ok(_) if !settled => Err(MetadataFailure::new(
            key,
            MetadataProblem::Owner("connection_unsettled"),
            MetadataPhaseOutcome::Unknown,
        )),
        Ok(affected) => Ok(affected),
    };
    match result {
        Ok(affected) => {
            completed.push(key);
            Ok(affected)
        }
        Err(mut error) => {
            error.completed_statements = completed.clone();
            Err(error)
        }
    }
}

fn item_failure(result: Result<MetadataExecution>, id: &str) -> Result<MetadataExecution> {
    result.map_err(|mut error| {
        error.item_id = Some(id.into());
        error
    })
}
