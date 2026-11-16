select id, paper_id, request_json from ai_paper_review_confirmation_batches
             where idempotency_key = ?1
