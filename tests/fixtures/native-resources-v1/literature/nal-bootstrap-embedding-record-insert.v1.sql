insert into embedding_records
         (id, representation_id, embedding_space_id, index_generation_id, lance_table,
          lance_row_id, vector_encoding, status, created_at, updated_at)
         values (?1, ?2, ?3, ?4, ?5, ?6, 'float32', 'queued',
                 strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
                 strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))