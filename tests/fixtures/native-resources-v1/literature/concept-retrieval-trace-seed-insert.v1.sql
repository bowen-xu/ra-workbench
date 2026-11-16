insert into concept_retrieval_trace_seeds
             (trace_id, concept_id, concept_revision, ordinal, lexical_rank, vector_rank,
              vector_distance, rrf_score, label, preferred_term, matched_by_json)
             values (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)