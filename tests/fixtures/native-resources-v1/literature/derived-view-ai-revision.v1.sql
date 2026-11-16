insert into derived_research_view_revisions
         (id, view_id, revision, snapshot_json, editor_kind, change_reason, created_at)
         values (?1, ?2, 1, ?3, ?4, 'accepted_ai_candidate',
                 strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
