insert into claims
             (id, project_id, statement, origin, provenance_kind, epistemic_role,
              revision, created_at, updated_at)
             values (?1, ?2, ?3, 'user', 'human_original', ?4, 1,
                     strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
                     strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
