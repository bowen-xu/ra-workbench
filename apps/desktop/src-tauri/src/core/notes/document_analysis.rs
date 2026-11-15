//! Owned projection of the existing Core Note engine for native consumers.
use super::{document_validation, project_catalog_document_text};
use research_workbench_native_sdk::note_document::{
    NoteDocument, NoteDocumentAnalysis, NoteDocumentAnalysisResult,
};

#[cfg(test)]
#[path = "document_analysis/tests.rs"]
mod tests;

/// Source-scoped asset identities are checked by the domain's finite rules.
pub(crate) fn analyze_with_external_assets(document: NoteDocument) -> NoteDocumentAnalysis {
    let mut known = std::collections::BTreeSet::new();
    let mut pending: Vec<_> = document.blocks.iter().collect();
    while let Some(block) = pending.pop() {
        if document_validation::known_block(&block.block_type) {
            known.insert(block.block_type.clone());
        }
        pending.extend(&block.children);
    }
    let result = match document_validation::validate_document_with_external_assets(&document) {
        Ok(()) => {
            let text = project_catalog_document_text(&document);
            NoteDocumentAnalysisResult::Ready {
                folded_text: research_note_query::note_fold(&text),
                text,
            }
        }
        Err(cause) => NoteDocumentAnalysisResult::Invalid { cause },
    };
    NoteDocumentAnalysis {
        document,
        known_block_types: known.into_iter().collect(),
        result,
    }
}
