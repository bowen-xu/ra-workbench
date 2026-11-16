insert into concept_retrieval_trace_edges
             (trace_id, relation_assertion_id, relation_revision, subject_concept_id,
              subject_label, predicate_kind, object_concept_id, object_label, hop, ordinal)
             values (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)