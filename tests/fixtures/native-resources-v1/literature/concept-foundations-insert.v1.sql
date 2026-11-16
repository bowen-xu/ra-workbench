insert into paper_foundation_similarity_results
             (run_id, ordinal, left_paper_id, left_paper_title, right_paper_id,
              right_paper_title, shared_concepts_json, weighted_jaccard,
              shared_concept_count, left_foundation_count, right_foundation_count)
             values (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
