insert into ai_evidence_bundles
         (id, run_id, scope_hash, retrieval_config_version, validation_status, created_at)
         values (?1, ?2, ?3, ?4, 'valid', strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))