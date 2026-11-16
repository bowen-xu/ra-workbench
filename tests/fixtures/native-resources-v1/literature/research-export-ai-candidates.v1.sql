select id, run_id, evidence_bundle_id, candidate_kind, schema_version,
                     current_revision, review_state, validation_status, original_payload_hash,
                     created_at, updated_at from ai_candidates order by id
