select id, object_kind, object_id, revision, snapshot_json, editor_kind,
                    change_reason, created_at
             from research_object_revisions
             where object_kind = ?1 and object_id = ?2
             order by revision desc
