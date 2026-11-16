select ordinal, concept_id, concept_revision, concept_label, paper_id, paper_title,
                issued_year, assertion_id, assertion_revision, paper_function, novelty_status,
                event_kind, explanation, evidence_anchor_id
         from concept_evolution_results where run_id = ?1 order by ordinal
