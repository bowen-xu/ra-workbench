select id, candidate_id, candidate_revision, suggestion_id, feedback_kind,
                     note, idempotency_key, created_at from ai_attention_feedback order by id
