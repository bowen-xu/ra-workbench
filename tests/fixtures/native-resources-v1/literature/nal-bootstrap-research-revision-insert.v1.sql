insert into research_object_revisions
         (id, object_kind, object_id, revision, snapshot_json, editor_kind, change_reason, created_at)
         values (?1, ?2, ?3, ?4, ?5, ?6, ?7, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))