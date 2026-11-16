select relation_assertion_id, relation_revision, subject_concept_id, subject_label,
                predicate_kind, object_concept_id, object_label, hop
         from concept_retrieval_trace_edges where trace_id = ?1 order by ordinal