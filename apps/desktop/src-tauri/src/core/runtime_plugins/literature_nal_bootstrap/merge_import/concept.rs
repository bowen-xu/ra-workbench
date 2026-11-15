use super::*;
use research_workbench_native_sdk::knowledge_snapshot::KnowledgeTerm;
use uuid::Uuid;

struct RevisionPolicy {
    provenance: String,
    kind: String,
    revision: i64,
    editor: String,
    reason: String,
}

impl Import<'_, '_> {
    pub(super) fn concept(
        &self,
        label: &str,
        explanation: &str,
        context: FailureContext,
    ) -> Result<String> {
        let io = Self { context, ..*self };
        io.create_concept(label, explanation)
    }

    fn create_concept(&self, label: &str, explanation: &str) -> Result<String> {
        let term = self.ensure_term(label)?;
        let name = self.text(
            Phase::ConceptNameCollision,
            &self.rules.legacy_name_resolve,
            vec![text(label)],
        )?;
        let mut details = self.first(
            Phase::ConceptValidation,
            &self.rules.legacy_concept_details,
            vec![V::Text(name), text(explanation)],
            &[
                C::OptionalString,
                C::OptionalString,
                C::String,
                C::String,
                C::String,
                C::String,
                C::String,
                C::String,
            ],
        )?;
        let rejection = details
            .optional_text()
            .map_err(|e| self.problem(Phase::ConceptValidation, e))?;
        details
            .optional_text()
            .map_err(|e| self.problem(Phase::ConceptValidation, e))?;
        if rejection.is_some() {
            return Err(self.invalid(Phase::ConceptValidation));
        }
        let mut fields = Vec::with_capacity(6);
        for _ in 0..6 {
            fields.push(
                details
                    .text()
                    .map_err(|e| self.problem(Phase::ConceptValidation, e))?,
            );
        }
        let unavailable = self
            .first(
                Phase::ConceptNameAvailability,
                &self.storage.concept_name_available,
                vec![text(&fields[0]), V::Null],
                &[C::Bool],
            )?
            .boolean()
            .map_err(|e| self.problem(Phase::ConceptNameAvailability, e))?;
        if unavailable {
            return Err(self.invalid(Phase::ConceptNameAvailability));
        }
        let policy = self.policy(Phase::ConceptInsert, &self.rules.legacy_concept_metadata)?;
        let id = Uuid::new_v4().to_string();
        let mut args = vec![text(&id)];
        args.extend(fields.into_iter().map(V::Text));
        args.push(text(&policy.provenance));
        self.write(Phase::ConceptInsert, &self.storage.concept_insert, args)?;
        self.write(
            Phase::PreferredLink,
            &self.storage.concept_preferred_insert,
            vec![text(&id), text(&term.id)],
        )?;
        let actual = self
            .storage
            .read_concept_with(self.tx, &id, |mappings| {
                let json = serde_json::to_string(mappings)
                    .map_err(|_| CaptureError::Shape("concept-mappings-codec"))?;
                let rows = super::super::rows::read(
                    self.tx,
                    &self.rules.legacy_preferred_term_id,
                    vec![V::Text(json)],
                    P::FirstOptional,
                    &[C::String],
                )?;
                super::super::capture::first(rows)?.text()
            })
            .map_err(|cause| self.problem(Phase::ConceptReadback, cause))?;
        // Actual trigger-provided fields, aliases, provenance and revision are
        // the snapshot authority. No prediction is compared against them.
        self.revision(Phase::ConceptRevision, &id, &policy, &actual)?;
        self.semantic(&actual)?;
        Ok(id)
    }

    fn ensure_term(&self, label: &str) -> Result<KnowledgeTerm> {
        let mut row = self.first(
            Phase::TermValidation,
            &self.rules.legacy_term_input,
            vec![text(label)],
            &[
                C::OptionalString,
                C::OptionalString,
                C::String,
                C::String,
                C::String,
            ],
        )?;
        let rejected = row
            .optional_text()
            .map_err(|e| self.problem(Phase::TermValidation, e))?;
        row.optional_text()
            .map_err(|e| self.problem(Phase::TermValidation, e))?;
        if rejected.is_some() {
            return Err(self.invalid(Phase::TermValidation));
        }
        let (trimmed, normalized, language) =
            (|| Ok::<_, CaptureError>((row.text()?, row.text()?, row.text()?)))()
                .map_err(|e| self.problem(Phase::TermValidation, e))?;
        let existing = self.optional(
            Phase::TermLookup,
            &self.storage.term_lookup,
            vec![text(&normalized), text(&language)],
        )?;
        if let Some(id) = existing {
            return self
                .storage
                .read_term_in(self.tx, &id)
                .map_err(|e| self.problem(Phase::TermReadback, e));
        }
        let policy = self.policy(Phase::TermInsert, &self.rules.legacy_term_metadata)?;
        let id = Uuid::new_v4().to_string();
        self.write(
            Phase::TermInsert,
            &self.storage.term_insert,
            vec![
                text(&id),
                V::Text(trimmed),
                V::Text(normalized),
                V::Text(language),
                text(&policy.provenance),
            ],
        )?;
        let actual = self
            .storage
            .read_term_in(self.tx, &id)
            .map_err(|e| self.problem(Phase::TermReadback, e))?;
        self.revision(Phase::TermRevision, &id, &policy, &actual)?;
        Ok(actual)
    }

    fn policy(&self, phase: Phase, resource: &AcceptedDataResource) -> Result<RevisionPolicy> {
        let mut row = self.first(
            phase,
            resource,
            vec![],
            &[C::String, C::String, C::I64, C::String, C::String],
        )?;
        (|| {
            Ok::<_, CaptureError>(RevisionPolicy {
                provenance: row.text()?,
                kind: row.text()?,
                revision: row.integer()?,
                editor: row.text()?,
                reason: row.text()?,
            })
        })()
        .map_err(|e| self.problem(phase, e))
    }

    fn revision(
        &self,
        phase: Phase,
        id: &str,
        policy: &RevisionPolicy,
        actual: &impl Serialize,
    ) -> Result<()> {
        // Preserve the original Value conversion before UUID allocation and
        // the string conversion afterwards, including its failure position.
        let value = serde_json::to_value(actual).map_err(|_| self.invalid(phase))?;
        let revision_id = Uuid::new_v4().to_string();
        let snapshot = self.encode(phase, &value)?;
        self.write(
            phase,
            &self.storage.research_revision_insert,
            vec![
                V::Text(revision_id),
                text(&policy.kind),
                text(id),
                V::Integer(policy.revision),
                V::Text(snapshot),
                text(&policy.editor),
                text(&policy.reason),
            ],
        )?;
        Ok(())
    }
}
