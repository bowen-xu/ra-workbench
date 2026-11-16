select id, candidate_id, candidate_revision, action, rationale, idempotency_key,
                     target_object_kind, target_object_id, target_object_revision, created_at
              from ai_review_decisions order by candidate_id, created_at
