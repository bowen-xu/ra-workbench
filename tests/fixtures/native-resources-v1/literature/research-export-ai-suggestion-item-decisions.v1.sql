select id, candidate_id, candidate_revision, suggestion_id, action,
                     resolution_kind, rationale, idempotency_key, request_hash,
                     target_object_kind, target_object_id, target_object_revision, created_at
              from ai_suggestion_item_decisions order by candidate_id, created_at
