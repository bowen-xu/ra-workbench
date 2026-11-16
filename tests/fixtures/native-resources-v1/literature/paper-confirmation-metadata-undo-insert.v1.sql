insert into ai_paper_review_metadata_confirmation_undos
         (id, confirmation_id, idempotency_key, created_at)
         values (?1, ?2, ?3, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
