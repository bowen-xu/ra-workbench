select id, view_id, revision, snapshot_json, editor_kind, change_reason, created_at
              from derived_research_view_revisions order by view_id, revision
