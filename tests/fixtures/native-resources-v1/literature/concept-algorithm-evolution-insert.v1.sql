insert into concept_evolution_results
             (run_id, ordinal, concept_id, concept_revision, concept_label, paper_id,
              paper_title, issued_year, assertion_id, assertion_revision, paper_function,
              novelty_status, event_kind, explanation, evidence_anchor_id)
             values (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)
