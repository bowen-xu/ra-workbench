insert into research_object_revisions
         (id, object_kind, object_id, revision, snapshot_json, editor_kind, change_reason, created_at)
         values (?1, 'claim', ?2, ?3, ?4, 'human', ?5,
                 strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
