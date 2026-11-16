insert into concept_retrieval_trace_sources
             (trace_id, anchor_id, concept_id, paper_id, source_kind, source_subject_id,
              source_subject_revision, paper_title, concept_label, quote_exact, context_text,
              page_index, page_label, ordinal, selected, omission_reason)
             values (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)