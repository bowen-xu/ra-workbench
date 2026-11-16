select b.id, b.paper_id, b.project_id, b.idempotency_key, b.created_at, u.created_at
         from ai_paper_review_confirmation_batches b
         left join ai_paper_review_confirmation_undos u on u.batch_id = b.id
         where b.id = ?1
