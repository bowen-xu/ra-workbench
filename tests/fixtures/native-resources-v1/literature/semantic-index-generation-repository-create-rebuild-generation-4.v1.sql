insert into embedding_index_generations
         (id, namespace, embedding_space_id, backend_kind, generation, status,
          relative_database_path, lance_table, index_kind, config_json, created_at)
         values (?1, ?2, ?3, 'lancedb', ?4, 'building', ?5, ?2, 'exact', '{}',
                 strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))