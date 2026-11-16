insert into paper_context_snapshots
         (id, paper_id, context_hash, context_schema_version, created_at)
         values (?1, ?2, ?3, ?4, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))