insert into term_usages
         (id, term_id, concept_id, anchor_id, usage_text, provenance_kind,
          revision, created_at, updated_at)
         values (?1, ?2, ?3, ?4, ?5, ?6, 1,
                 strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
                 strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
