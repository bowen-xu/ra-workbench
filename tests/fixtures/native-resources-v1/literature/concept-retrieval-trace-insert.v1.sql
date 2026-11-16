insert into concept_retrieval_traces
         (id, paper_id, query_sha256, embedding_space_id, index_generation_id, status,
          config_json, context_text, context_sha256, seed_count, edge_count, source_count,
          external_paper_count, created_at, expires_at)
         values (?1, ?2, ?3, ?4, ?5, 'previewed', ?6, ?7, ?8, ?9, ?10, ?11, ?12,
                 strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
                 strftime('%Y-%m-%dT%H:%M:%fZ', 'now', '+15 minutes'))