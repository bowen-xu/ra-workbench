//! Fixed concept search/save adapter. No database or resource executor is public.
pub(crate) mod commands;
mod data;
mod execute;
mod failure;
mod resources;
pub(crate) mod semantic;
pub(crate) use execute::{apply, check_preparation};
pub(crate) use failure::{ConceptFailure, ConceptOutcome, ConceptProblem};
use research_workbench_native_sdk::concepts::Concept;
pub(crate) use research_workbench_native_sdk::concepts::ConceptRequestV1 as ConceptRequest;
pub(crate) use resources::ConceptResources;
pub(crate) enum ConceptResult {
    Concepts(Vec<Concept>),
    Concept(Box<Concept>),
}

#[cfg(test)]
mod tests;

pub(crate) fn register_accepted_semantic(
    tx: &rusqlite::Transaction<'_>,
    resources: &ConceptResources,
    concept: &research_workbench_native_sdk::concepts::Concept,
) -> Result<(), ConceptFailure> {
    execute::semantic(tx, resources, concept).map_err(|mut failure| {
        failure.concept_id = Some(concept.id.clone());
        failure
    })
}
