insert into paper_concept_assertion_revisions
         (id, assertion_id, revision, paper_function, novelty_status, explanation_status,
          explanation, role_rationale, novelty_rationale, source_candidate_id,
          source_candidate_revision, source_local_concept_id, created_at)
         values (?1, ?2, 1, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11,
                 strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
