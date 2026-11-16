select id, reference_record_id, revision, snapshot_json, editor_kind,
                     change_reason, created_at from reference_record_revisions
              order by reference_record_id, revision
