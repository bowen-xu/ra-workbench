insert into evidence_matrix_revisions
         (id, matrix_id, revision, snapshot_json, editor_kind, change_reason, created_at)
         values (?1, ?2, ?3, ?4, 'human', ?5,
                 strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
