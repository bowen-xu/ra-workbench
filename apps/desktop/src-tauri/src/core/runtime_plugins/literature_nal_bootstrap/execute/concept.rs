use super::Execution;
use crate::{
    data_interface::plugin_resources::{ColumnCodec as C, RowPolicy as P},
    runtime_plugins::literature_nal_bootstrap::{
        error::CaptureError,
        metadata::{TERM_CODECS, decode_term},
        outcome::{BootstrapCause, BootstrapProblem, FailureContext},
        rows::Row,
    },
};
use research_workbench_native_sdk::{
    data::DataValue as V, knowledge_snapshot::KnowledgeTerm, nal_bootstrap::*,
};
use serde::Serialize;

impl Execution<'_, '_> {
    pub(super) fn concept(
        &mut self,
        key: &ConceptCallKeyV1,
        context: FailureContext,
    ) -> Result<String, BootstrapProblem> {
        let previous = std::mem::replace(&mut self.context, context);
        let result = self.execute_concept(key);
        self.context = previous;
        result
    }

    fn execute_concept(&mut self, key: &ConceptCallKeyV1) -> Result<String, BootstrapProblem> {
        let intent = self
            .input
            .probes
            .concepts
            .iter()
            .find(|value| &value.key == key)
            .cloned()
            .ok_or_else(|| {
                BootstrapProblem::binding(BootstrapPhaseV1::TermValidation, "concept-intent")
            })?;
        let slots = self
            .input
            .identities
            .concepts
            .iter()
            .find(|value| &value.key == key)
            .cloned()
            .ok_or_else(|| {
                BootstrapProblem::binding(BootstrapPhaseV1::TermValidation, "concept-slots")
            })?;
        let prepared = self
            .plan
            .concepts
            .iter()
            .find(|value| &value.key == key)
            .cloned()
            .ok_or_else(|| {
                BootstrapProblem::binding(BootstrapPhaseV1::TermValidation, "concept-preparation")
            })?;
        let term = self.ensure_term(&intent, &slots, &prepared)?;
        let resources = self.resources;
        self.captured
            .require_phase_in(self.db, BootstrapPhaseV1::ConceptNameCollision)
            .map_err(|error| self.problem(BootstrapPhaseV1::ConceptNameCollision, error))?;
        if prepared.name_checks.is_empty() {
            return Err(BootstrapProblem::binding(
                BootstrapPhaseV1::ConceptNameCollision,
                "name-checks",
            ));
        }
        for (index, check) in prepared.name_checks.iter().enumerate() {
            let rows = self.read(
                BootstrapPhaseV1::ConceptNameCollision,
                &resources.concept_name_exists,
                vec![V::Text(check.candidate.clone())],
                P::FirstRequired,
                &[C::Bool],
            )?;
            let actual = boolean(rows)
                .map_err(|error| self.problem(BootstrapPhaseV1::ConceptNameCollision, error))?;
            if actual != check.exists {
                return Err(self.problem(
                    BootstrapPhaseV1::ConceptNameCollision,
                    CaptureError::changed(None),
                ));
            }
            if !actual && index + 1 != prepared.name_checks.len() {
                return Err(BootstrapProblem::binding(
                    BootstrapPhaseV1::ConceptNameCollision,
                    "name-check-after-first-available",
                ));
            }
        }
        if prepared
            .name_checks
            .last()
            .is_some_and(|check| check.exists)
        {
            return Err(BootstrapProblem::binding(
                BootstrapPhaseV1::ConceptNameCollision,
                "unfinished-name-checks",
            ));
        }
        let SourceProbe::Available { value: details } = &prepared.details else {
            return Err(BootstrapProblem::binding(
                BootstrapPhaseV1::ConceptValidation,
                "concept-details",
            ));
        };
        if let Some(reason) = details.rejection {
            return Err(self.concept_reject(BootstrapPhaseV1::ConceptValidation, reason));
        }
        let rows = self.read(
            BootstrapPhaseV1::ConceptNameAvailability,
            &resources.concept_name_available,
            vec![V::Text(details.label.clone()), V::Null],
            P::FirstRequired,
            &[C::Bool],
        )?;
        let exists = boolean(rows)
            .map_err(|error| self.problem(BootstrapPhaseV1::ConceptNameAvailability, error))?;
        if exists == details.available {
            return Err(self.problem(
                BootstrapPhaseV1::ConceptNameAvailability,
                CaptureError::changed(None),
            ));
        }
        if !details.available {
            return Err(self.concept_reject(
                BootstrapPhaseV1::ConceptNameAvailability,
                BootstrapRejectionV1::NameUnavailable,
            ));
        }
        if prepared.concept_id != slots.concept_id {
            return Err(BootstrapProblem::binding(
                BootstrapPhaseV1::ConceptInsert,
                "concept-id",
            ));
        }
        self.write(
            BootstrapPhaseV1::ConceptInsert,
            &resources.concept_insert,
            vec![
                V::Text(slots.concept_id.clone()),
                V::Text(details.label.clone()),
                V::Text(details.explanation.clone()),
                V::Text(details.disambiguation.clone()),
                V::Text(details.domain.clone()),
                V::Text(details.tradition.clone()),
                V::Text(details.period.clone()),
                V::Text("human_original".into()),
            ],
        )?;
        self.write(
            BootstrapPhaseV1::PreferredLink,
            &resources.concept_preferred_insert,
            vec![V::Text(slots.concept_id.clone()), V::Text(term.id.clone())],
        )?;
        let actual = resources
            .read_concept_in(self.db, &slots.concept_id)
            .map_err(|error| self.problem(BootstrapPhaseV1::ConceptReadback, error))?;
        if actual.id != slots.concept_id
            || actual.label != details.label
            || actual.explanation != details.explanation
            || actual.disambiguation != details.disambiguation
            || actual.domain != details.domain
            || actual.tradition != details.tradition
            || actual.period != details.period
            || actual.provenance_kind != "human_original"
            || actual.revision != 1
            || !same_term_fields(&actual.preferred_term, &term)
            || actual.terms.len() != 1
            || !same_term_fields(&actual.terms[0], &term)
        {
            return Err(self.problem(
                BootstrapPhaseV1::ConceptReadback,
                CaptureError::changed(None),
            ));
        }
        if prepared.concept_revision_id != slots.concept_revision_id {
            return Err(BootstrapProblem::binding(
                BootstrapPhaseV1::ConceptRevision,
                "concept-revision-id",
            ));
        }
        self.save_snapshot(
            BootstrapPhaseV1::ConceptRevision,
            &slots.concept_revision_id,
            "concept",
            &slots.concept_id,
            "nal_collection_migration",
            &actual,
        )?;
        self.semantic(&prepared, &actual, &slots)?;
        Ok(slots.concept_id)
    }

    fn ensure_term(
        &mut self,
        intent: &ConceptIntentV1,
        slots: &ConceptIdentitySlotsV1,
        prepared: &PreparedConceptV1,
    ) -> Result<KnowledgeTerm, BootstrapProblem> {
        let TermValidationV1::Prepared {
            trimmed_text,
            normalized_text,
            language,
        } = &intent.term
        else {
            let TermValidationV1::Rejected { reason } = &intent.term else {
                return Err(BootstrapProblem::binding(
                    BootstrapPhaseV1::TermValidation,
                    "term-validation",
                ));
            };
            return Err(self.concept_reject(BootstrapPhaseV1::TermValidation, *reason));
        };
        let resources = self.resources;
        let lookup = self.read(
            BootstrapPhaseV1::TermLookup,
            &resources.term_lookup,
            vec![V::Text(normalized_text.clone()), V::Text(language.clone())],
            P::FirstOptional,
            &[C::String],
        )?;
        let existing = lookup
            .rows
            .into_iter()
            .next()
            .map(|row| Row::from(row).text())
            .transpose()
            .map_err(|error| self.problem(BootstrapPhaseV1::TermLookup, error))?;
        if let Some(id) = existing {
            let actual = self.read_term(&id)?;
            let matches = match &prepared.term {
                SourceProbe::Available {
                    value: TermResolutionV1::Existing { term },
                } => same_term_fields(&actual, term),
                SourceProbe::Available {
                    value:
                        TermResolutionV1::Planned {
                            id,
                            text,
                            normalized_text,
                            language,
                        },
                } => {
                    self.input
                        .identities
                        .concepts
                        .iter()
                        .any(|slots| &slots.term_id == id)
                        && actual.id == *id
                        && actual.text == *text
                        && actual.normalized_text == *normalized_text
                        && actual.language == *language
                        && actual.provenance_kind == "human_original"
                        && actual.revision == 1
                }
                _ => false,
            };
            if !matches {
                return Err(
                    self.problem(BootstrapPhaseV1::TermReadback, CaptureError::changed(None))
                );
            }
            return Ok(actual);
        }
        let SourceProbe::Available {
            value:
                TermResolutionV1::Create {
                    id,
                    text,
                    normalized_text: normalized,
                    language: prepared_language,
                    revision_id,
                },
        } = &prepared.term
        else {
            return Err(BootstrapProblem::binding(
                BootstrapPhaseV1::TermInsert,
                "term-create",
            ));
        };
        if id != &slots.term_id
            || text != trimmed_text
            || normalized != normalized_text
            || prepared_language != language
        {
            return Err(BootstrapProblem::binding(
                BootstrapPhaseV1::TermInsert,
                "term-create-binding",
            ));
        }
        self.write(
            BootstrapPhaseV1::TermInsert,
            &resources.term_insert,
            vec![
                V::Text(id.clone()),
                V::Text(text.clone()),
                V::Text(normalized.clone()),
                V::Text(prepared_language.clone()),
                V::Text("human_original".into()),
            ],
        )?;
        let actual = self.read_term(id)?;
        if actual.id != *id
            || actual.text != *text
            || actual.normalized_text != *normalized
            || actual.language != *prepared_language
            || actual.provenance_kind != "human_original"
            || actual.revision != 1
        {
            return Err(self.problem(BootstrapPhaseV1::TermReadback, CaptureError::changed(None)));
        }
        if revision_id != &slots.term_revision_id {
            return Err(BootstrapProblem::binding(
                BootstrapPhaseV1::TermRevision,
                "term-revision-id",
            ));
        }
        self.save_snapshot(
            BootstrapPhaseV1::TermRevision,
            revision_id,
            "term",
            id,
            "manual_create",
            &actual,
        )?;
        Ok(actual)
    }

    fn read_term(&mut self, id: &str) -> Result<KnowledgeTerm, BootstrapProblem> {
        let resources = self.resources;
        let rows = self.read(
            BootstrapPhaseV1::TermReadback,
            &resources.term_read,
            vec![V::Text(id.into())],
            P::FirstOptional,
            TERM_CODECS,
        )?;
        decode_term(rows).map_err(|error| self.problem(BootstrapPhaseV1::TermReadback, error))
    }

    fn save_snapshot(
        &mut self,
        phase: BootstrapPhaseV1,
        revision_id: &str,
        kind: &str,
        id: &str,
        reason: &str,
        value: &impl Serialize,
    ) -> Result<(), BootstrapProblem> {
        // Preserve the original two-step Value conversion, governed by this
        // caller's real serde_json features and actual SQLite metadata.
        let snapshot = serde_json::to_value(value)
            .and_then(|value| serde_json::to_string(&value))
            .map_err(|_| self.concept_reject(phase, BootstrapRejectionV1::InvalidStructure))?;
        let resources = self.resources;
        self.write(
            phase,
            &resources.research_revision_insert,
            vec![
                V::Text(revision_id.into()),
                V::Text(kind.into()),
                V::Text(id.into()),
                V::Integer(1),
                V::Text(snapshot),
                V::Text("human".into()),
                V::Text(reason.into()),
            ],
        )?;
        Ok(())
    }

    pub(super) fn concept_reject(
        &self,
        phase: BootstrapPhaseV1,
        reason: BootstrapRejectionV1,
    ) -> BootstrapProblem {
        BootstrapProblem {
            phase,
            context: self.context,
            cause: BootstrapCause::Rejected(reason),
        }
    }
}

fn boolean(rows: research_workbench_native_sdk::data::DataRows) -> Result<bool, CaptureError> {
    rows.rows
        .into_iter()
        .next()
        .ok_or(CaptureError::Shape("required-row"))
        .and_then(|row| Row::from(row).boolean())
}
fn same_term_fields(left: &KnowledgeTerm, right: &KnowledgeTerm) -> bool {
    left.id == right.id
        && left.text == right.text
        && left.normalized_text == right.normalized_text
        && left.language == right.language
        && left.provenance_kind == right.provenance_kind
        && left.revision == right.revision
}
