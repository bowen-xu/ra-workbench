select id, candidate_id, revision, payload_json, payload_hash, editor_kind,
                     source_run_id, parent_revision, change_reason, created_at
              from ai_candidate_revisions order by candidate_id, revision
