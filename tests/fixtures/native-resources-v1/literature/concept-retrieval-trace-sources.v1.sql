select anchor_id, concept_id, concept_label, paper_id, paper_title, source_kind,
                source_subject_id, source_subject_revision, page_index, page_label,
                quote_exact, context_text, selected, omission_reason
         from concept_retrieval_trace_sources where trace_id = ?1 order by ordinal