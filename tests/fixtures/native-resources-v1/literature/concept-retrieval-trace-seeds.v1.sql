select concept_id, concept_revision, label, preferred_term, matched_by_json,
                lexical_rank, vector_rank, vector_distance, rrf_score
         from concept_retrieval_trace_seeds where trace_id = ?1 order by ordinal