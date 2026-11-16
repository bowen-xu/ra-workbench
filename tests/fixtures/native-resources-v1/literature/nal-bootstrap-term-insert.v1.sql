insert into terms
         (id, text, normalized_text, language, provenance_kind, revision, created_at, updated_at)
         values (?1, ?2, ?3, ?4, ?5, 1,
                 strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
                 strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))