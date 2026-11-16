insert into paper_concept_assertions
         (id, paper_id, concept_id, term_usage_id, current_revision, created_at, updated_at)
         values (?1, ?2, ?3, ?4, 1, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
                 strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
