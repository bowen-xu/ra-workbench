insert into evidence_matrices
         (id, project_id, title, paper_ids_json, dimensions_json, current_revision,
          created_at, updated_at)
         values (?1, ?2, ?3, ?4, ?5, 1,
                 strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
                 strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
