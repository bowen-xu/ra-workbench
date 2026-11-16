select ordinal, left_paper_id, left_paper_title, right_paper_id, right_paper_title,
                shared_concepts_json, weighted_jaccard, shared_concept_count,
                left_foundation_count, right_foundation_count
         from paper_foundation_similarity_results where run_id = ?1 order by ordinal
