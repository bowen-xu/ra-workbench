use super::*;
use research_workbench_native_sdk::nal_state::NalTerm;
use std::collections::HashMap;
use uuid::Uuid;

struct Fact {
    subject: String,
    predicate: String,
    relation: String,
    origin: String,
}

impl Import<'_, '_> {
    fn seed(
        &self,
        phase: Phase,
        resource: &AcceptedDataResource,
        args: Vec<V>,
        context: FailureContext,
    ) -> Result<Option<String>> {
        let rows = self.read(phase, resource, args, P::FirstOptional, &[C::String; 2])?;
        let Some(row) = rows.rows.into_iter().next() else {
            return Ok(None);
        };
        let mut row = Row::from(row);
        let (label, explanation) = (|| Ok::<_, CaptureError>((row.text()?, row.text()?)))()
            .map_err(|e| self.problem(phase, e))?;
        self.concept(&label, &explanation, context).map(Some)
    }

    fn required_seed(
        &self,
        phase: Phase,
        resource: &AcceptedDataResource,
        args: Vec<V>,
        context: FailureContext,
    ) -> Result<String> {
        self.seed(phase, resource, args, context)?
            .ok_or_else(|| self.invalid(phase))
    }

    fn fact(
        &self,
        phase: Phase,
        resource: &AcceptedDataResource,
        args: Vec<V>,
    ) -> Result<Option<Fact>> {
        let rows = self.read(phase, resource, args, P::FirstOptional, &[C::String; 4])?;
        rows.rows
            .into_iter()
            .next()
            .map(|row| {
                let mut row = Row::from(row);
                (|| {
                    Ok::<_, CaptureError>(Fact {
                        subject: row.text()?,
                        predicate: row.text()?,
                        relation: row.text()?,
                        origin: row.text()?,
                    })
                })()
                .map_err(|e| self.problem(phase, e))
            })
            .transpose()
    }

    fn insert(&self, phase: Phase, fact: &Fact) -> Result<()> {
        let id = Uuid::new_v4().to_string();
        self.write(
            phase,
            &self.storage.statement_relation_kind,
            vec![text(&fact.relation)],
        )?;
        self.write(
            phase,
            &self.storage.statement_insert,
            vec![
                V::Text(id),
                text(&fact.subject),
                text(&fact.predicate),
                text(&fact.origin),
            ],
        )?;
        Ok(())
    }

    pub(super) fn legacy(&self) -> Result<()> {
        let done = self
            .first(
                Phase::LegacyHeader,
                &self.storage.legacy_header,
                vec![],
                &[C::Bool],
            )?
            .boolean()
            .map_err(|e| self.problem(Phase::LegacyHeader, e))?;
        if done {
            return Ok(());
        }
        // Freeze the complete original collection batch before any concept
        // write. Later source batches deliberately remain later actual reads.
        let collections = self.read(
            Phase::Collections,
            &self.storage.legacy_collections,
            vec![],
            P::All,
            &[C::String, C::String, C::String, C::OptionalString],
        )?;
        if !collections.rows.is_empty() {
            let membership = self.required_seed(
                Phase::Collections,
                &self.rules.legacy_membership_seed,
                vec![V::Integer(1)],
                FailureContext::Import,
            )?;
            let tagged = self.required_seed(
                Phase::Collections,
                &self.rules.legacy_tagged_seed,
                vec![V::Integer(1)],
                FailureContext::Import,
            )?;
            let mut ids = HashMap::new();
            let mut linked = Vec::new();
            for row in collections.rows {
                let mut row = Row::from(row);
                let (id, label, description, existing) = (|| {
                    Ok::<_, CaptureError>((
                        row.text()?,
                        row.text()?,
                        row.text()?,
                        row.optional_text()?,
                    ))
                })()
                .map_err(|e| self.problem(Phase::Collections, e))?;
                let target = self.required_seed(
                    Phase::Collections,
                    &self.rules.legacy_collection_seed,
                    vec![V::Text(label), V::Text(description)],
                    FailureContext::Import,
                )?;
                if let Some(existing) = existing {
                    linked.push((target.clone(), existing));
                }
                ids.insert(id, target);
            }
            if !linked.is_empty() {
                let predicate = self.required_seed(
                    Phase::Collections,
                    &self.rules.legacy_linked_seed,
                    vec![V::Integer(1)],
                    FailureContext::Import,
                )?;
                for (a, b) in linked {
                    let fact = self
                        .fact(
                            Phase::Collections,
                            &self.rules.legacy_linked_statement,
                            vec![V::Text(a), V::Text(b), text(&predicate)],
                        )?
                        .ok_or_else(|| self.invalid(Phase::Collections))?;
                    self.insert(Phase::Collections, &fact)?;
                }
            }
            let types = self.read(
                Phase::RelationTypes,
                &self.storage.legacy_types,
                vec![],
                P::All,
                &[C::String, C::String, C::String, C::Bool],
            )?;
            let mut predicates = HashMap::new();
            for row in types.rows {
                let mut row = Row::from(row);
                let (id, label, inverse, directed) = (|| {
                    Ok::<_, CaptureError>((row.text()?, row.text()?, row.text()?, row.boolean()?))
                })()
                .map_err(|e| self.problem(Phase::RelationTypes, e))?;
                let probe = self
                    .first(
                        Phase::RelatedFieldExistence,
                        &self.rules.legacy_type_probe_required,
                        vec![text(&id)],
                        &[C::Bool],
                    )?
                    .boolean()
                    .map_err(|e| self.problem(Phase::RelatedFieldExistence, e))?;
                let used = if probe {
                    V::Integer(i64::from(
                        self.first(
                            Phase::RelatedFieldExistence,
                            &self.storage.legacy_related_used,
                            vec![text(&id)],
                            &[C::Bool],
                        )?
                        .boolean()
                        .map_err(|e| self.problem(Phase::RelatedFieldExistence, e))?,
                    ))
                } else {
                    V::Null
                };
                if let Some(p) = self.seed(
                    Phase::RelationTypes,
                    &self.rules.legacy_type_seed,
                    vec![
                        text(&id),
                        V::Text(label),
                        V::Text(inverse),
                        V::Integer(i64::from(directed)),
                        used,
                    ],
                    FailureContext::Import,
                )? {
                    predicates.insert(id, (p, directed));
                }
            }
            let rows = self.read(
                Phase::Relations,
                &self.storage.legacy_relations,
                vec![],
                P::All,
                &[C::String; 4],
            )?;
            let relations = rows
                .rows
                .into_iter()
                .map(|row| {
                    let mut row = Row::from(row);
                    (||Ok::<_,CaptureError>((row.text()?,row.text()?,row.text()?,row.text()?)))()
                    .map_err(|e|self.problem(Phase::Relations,e))
                })
                .collect::<Result<Vec<_>>>()?;
            let inheritance = self.seed(
                Phase::Relations,
                &self.rules.legacy_inheritance_seed,
                vec![V::Text(self.encode(Phase::Relations, &relations)?)],
                FailureContext::Import,
            )?;
            let mut relation_terms = HashMap::new();
            for (id, source, target, type_id) in relations {
                let a = ids
                    .get(&source)
                    .ok_or_else(|| self.invalid(Phase::Relations))?;
                let b = ids
                    .get(&target)
                    .ok_or_else(|| self.invalid(Phase::Relations))?;
                let p = predicates.get(&type_id);
                let forward = self.fact(
                    Phase::Relations,
                    &self.rules.legacy_relation_forward_statement,
                    vec![
                        text(a),
                        text(b),
                        text(&type_id),
                        nullable(inheritance.as_deref()),
                        nullable(p.map(|p| p.0.as_str())),
                    ],
                )?;
                if let Some(forward) = forward {
                    self.insert(Phase::Relations, &forward)?;
                    if let Some(reverse) = self.fact(
                        Phase::Relations,
                        &self.rules.legacy_relation_reverse_statement,
                        vec![
                            text(a),
                            text(b),
                            text(&type_id),
                            nullable(p.map(|p| p.0.as_str())),
                            p.map(|p| V::Integer(i64::from(p.1))).unwrap_or(V::Null),
                        ],
                    )? {
                        self.insert(Phase::Relations, &reverse)?;
                    }
                    relation_terms.insert(id, (forward.subject, forward.relation));
                } else {
                    return Err(self.invalid(Phase::Relations));
                }
            }
            let members = self.read(
                Phase::Memberships,
                &self.storage.legacy_memberships,
                vec![],
                P::All,
                &[C::String; 2],
            )?;
            for row in members.rows {
                let mut row = Row::from(row);
                let (paper, collection) = (|| Ok::<_, CaptureError>((row.text()?, row.text()?)))()
                    .map_err(|e| self.problem(Phase::Memberships, e))?;
                let target = ids
                    .get(&collection)
                    .ok_or_else(|| self.invalid(Phase::Memberships))?;
                let fact = self
                    .fact(
                        Phase::Memberships,
                        &self.rules.legacy_membership_statement,
                        vec![V::Text(paper), text(target), text(&membership)],
                    )?
                    .ok_or_else(|| self.invalid(Phase::Memberships))?;
                self.insert(Phase::Memberships, &fact)?;
            }
            let assignments = self.read(
                Phase::Assignments,
                &self.storage.legacy_assignments,
                vec![],
                P::All,
                &[C::String; 2],
            )?;
            for row in assignments.rows {
                let mut row = Row::from(row);
                let (paper, relation) = (|| Ok::<_, CaptureError>((row.text()?, row.text()?)))()
                    .map_err(|e| self.problem(Phase::Assignments, e))?;
                let (pair, predicate) = relation_terms
                    .get(&relation)
                    .ok_or_else(|| self.invalid(Phase::Assignments))?;
                let fact = self
                    .fact(
                        Phase::Assignments,
                        &self.rules.legacy_assignment_statement,
                        vec![V::Text(paper), text(pair), text(predicate), text(&tagged)],
                    )?
                    .ok_or_else(|| self.invalid(Phase::Assignments))?;
                self.insert(Phase::Assignments, &fact)?;
            }
        }
        self.write(Phase::ImportFlag, &self.storage.legacy_import_flag, vec![])?;
        Ok(())
    }

    pub(super) fn repair(&self) -> Result<()> {
        let version = self
            .first(
                Phase::RepairVersion,
                &self.storage.repair_version,
                vec![],
                &[C::I64],
            )?
            .integer()
            .map_err(|e| self.problem(Phase::RepairVersion, e))?;
        let skip = self
            .first(
                Phase::RepairVersion,
                &self.rules.legacy_repair_skip,
                vec![V::Integer(version)],
                &[C::Bool],
            )?
            .boolean()
            .map_err(|e| self.problem(Phase::RepairVersion, e))?;
        if skip {
            return Ok(());
        }
        let records = self.read(
            Phase::RepairRecords,
            &self.storage.repair_records,
            vec![],
            P::All,
            &[C::String; 3],
        )?;
        if !records.rows.is_empty() {
            let relation = self.required_seed(
                Phase::RepairRole,
                &self.rules.legacy_repair_seed,
                vec![V::Integer(1)],
                FailureContext::Repair,
            )?;
            self.write(
                Phase::RepairRole,
                &self.storage.repair_relation_kind,
                vec![text(&relation)],
            )?;
            for row in records.rows {
                let mut row = Row::from(row);
                let (id, subject, predicate) =
                    (|| Ok::<_, CaptureError>((row.text()?, row.text()?, row.text()?)))()
                        .map_err(|e| self.problem(Phase::RepairRecords, e))?;
                let subject: NalTerm = serde_json::from_str(&subject)
                    .map_err(|_| self.invalid(Phase::RepairTermDecode))?;
                let predicate: NalTerm = serde_json::from_str(&predicate)
                    .map_err(|_| self.invalid(Phase::RepairTermDecode))?;
                let mut wrapped = self.first(
                    Phase::RepairTermDecode,
                    &self.rules.legacy_repair_wrap,
                    vec![
                        V::Text(self.encode(Phase::RepairTermDecode, &subject)?),
                        V::Text(self.encode(Phase::RepairTermDecode, &predicate)?),
                        text(&relation),
                    ],
                    &[C::String; 2],
                )?;
                let (subject, predicate) =
                    (|| Ok::<_, CaptureError>((wrapped.text()?, wrapped.text()?)))()
                        .map_err(|e| self.problem(Phase::RepairTermDecode, e))?;
                // No early decode of this new wrapper: the original next
                // complete load retains the stored bare-Term depth check.
                self.write(
                    Phase::RepairTermDecode,
                    &self.storage.repair_statement,
                    vec![V::Text(subject), V::Text(predicate), V::Text(id)],
                )?;
            }
        }
        self.write(Phase::RepairFlag, &self.storage.repair_flag, vec![])?;
        Ok(())
    }

    pub(super) fn cleanup(&self) -> Result<()> {
        let exists = self
            .first(
                Phase::CleanupExistence,
                &self.storage.cleanup_exists,
                vec![],
                &[C::Bool],
            )?
            .boolean()
            .map_err(|e| self.problem(Phase::CleanupExistence, e))?;
        if !exists {
            return Ok(());
        }
        let loaded = self.complete(Phase::CleanupLoad)?;
        let candidates = self.read(
            Phase::CleanupLoad,
            &self.rules.legacy_cleanup_select,
            vec![
                V::Text(self.encode(Phase::CleanupLoad, &loaded.raw.concepts)?),
                V::Text(self.encode(Phase::CleanupLoad, &loaded.statements)?),
                V::Text(self.encode(Phase::CleanupLoad, &loaded.raw.annotations)?),
                V::Text(self.encode(Phase::CleanupLoad, &loaded.raw.relation_concept_ids)?),
            ],
            P::All,
            &[C::String],
        )?;
        for row in candidates.rows {
            let id = Row::from(row)
                .text()
                .map_err(|e| self.problem(Phase::CleanupDelete, e))?;
            let affected = self.write(
                Phase::CleanupDelete,
                &self.storage.concept_delete,
                vec![text(&id)],
            )?;
            let affected =
                i64::try_from(affected).map_err(|_| self.invalid(Phase::CleanupDelete))?;
            let rejected = self
                .first(
                    Phase::CleanupDelete,
                    &self.rules.legacy_concept_delete_error,
                    vec![V::Integer(affected)],
                    &[C::OptionalString],
                )?
                .optional_text()
                .map_err(|e| self.problem(Phase::CleanupDelete, e))?;
            if rejected.is_some() {
                return Err(BootstrapProblem {
                    phase: Phase::CleanupDelete,
                    context: FailureContext::Network,
                    cause: BootstrapCause::MissingConcept,
                });
            }
            self.write(
                Phase::CleanupDelete,
                &self.storage.concept_search_delete,
                vec![V::Text(id)],
            )?;
        }
        Ok(())
    }
}

fn nullable(value: Option<&str>) -> V {
    value.map(text).unwrap_or(V::Null)
}
