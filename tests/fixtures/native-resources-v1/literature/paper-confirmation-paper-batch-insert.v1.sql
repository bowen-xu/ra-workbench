insert into ai_paper_review_confirmation_batches
                 (id, paper_id, project_id, request_json, idempotency_key, created_at)
                 values (?1, ?2, ?3, ?4, ?5, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
