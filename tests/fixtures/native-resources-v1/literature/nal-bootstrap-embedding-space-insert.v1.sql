insert into embedding_spaces
         (id, provider_id, region, model_id, model_revision, dimension, scalar_type,
          distance_metric, normalization, document_input_role, query_input_role,
          instruction_set_version, created_at)
         values (?1, 'qwen-embedding', ?2, ?3, ?3, ?4, 'float32', 'cosine', 'l2',
                 'document', 'query', ?5, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         on conflict(id) do nothing