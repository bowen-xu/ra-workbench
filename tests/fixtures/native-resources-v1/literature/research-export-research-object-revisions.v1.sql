select id, object_kind, object_id, revision, snapshot_json, editor_kind,
                     change_reason, created_at from research_object_revisions
              order by object_kind, object_id, revision
