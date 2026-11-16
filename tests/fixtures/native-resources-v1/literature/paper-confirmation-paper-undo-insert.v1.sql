insert or ignore into ai_paper_review_confirmation_undos
             (id, batch_id, idempotency_key, created_at)
             values (?1, ?2, ?3, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
