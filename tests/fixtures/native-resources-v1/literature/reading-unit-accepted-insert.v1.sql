insert into structured_reading_units
         (id, project_id, paper_id, unit_kind, title, body, provenance_kind,
          epistemic_role, revision, created_at, updated_at)
         values (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 1,
                 strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
                 strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
