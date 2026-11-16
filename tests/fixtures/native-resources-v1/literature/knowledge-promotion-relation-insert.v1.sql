insert into relation_assertions
         (id, project_id, subject_kind, subject_id, subject_label_snapshot, predicate_kind,
          object_kind, object_id, object_label_snapshot, scope_note, provenance_kind,
          epistemic_role, revision, created_at, updated_at)
         values (?1, ?2, 'concept', ?3, ?4, ?5, 'concept', ?6, ?7, ?8, ?9,
                 'assertion', 1, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
                 strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
