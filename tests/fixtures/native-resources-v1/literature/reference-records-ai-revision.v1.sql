insert into reference_record_revisions
         (id, reference_record_id, revision, snapshot_json, editor_kind,
          change_reason, created_at)
         values (?1, ?2, 1, ?3, 'ai_review', 'accepted_ai_candidate',
                 strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
