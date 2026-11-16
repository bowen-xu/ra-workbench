select id, paper_id, context_hash, context_schema_version, created_at
             from paper_context_snapshots where id = ?1