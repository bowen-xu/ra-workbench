insert into concept_algorithm_runs
         (id, algorithm_kind, algorithm_id, algorithm_version, parameters_json, parameters_hash,
          concept_revision_watermark, relation_revision_watermark, paper_scope_json,
          paper_scope_hash, status, created_at, started_at)
         values (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 'running',
                 strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
                 strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
