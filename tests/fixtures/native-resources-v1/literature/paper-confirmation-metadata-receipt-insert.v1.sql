insert into ai_paper_review_metadata_confirmations
         (id, batch_id, field_name, previous_confirmed_by_user, previous_updated_at,
          applied_updated_at, created_at)
         values (?1, ?2, ?3, ?4, ?5, ?6, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
