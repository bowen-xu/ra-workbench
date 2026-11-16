update relation_assertions set project_id = ?2, subject_kind = ?3, subject_id = ?4,
             subject_label_snapshot = ?5, predicate_kind = ?6, object_kind = ?7,
             object_id = ?8, object_label_snapshot = ?9, scope_note = ?10, revision = ?11,
             updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         where id = ?1 and revision = ?12
