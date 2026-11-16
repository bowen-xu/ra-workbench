select id, project_id, subject_kind, subject_id, subject_label_snapshot,
                     predicate_kind, object_kind, object_id, object_label_snapshot, scope_note,
                     provenance_kind, epistemic_role, revision, created_at, updated_at
              from relation_assertions order by id
