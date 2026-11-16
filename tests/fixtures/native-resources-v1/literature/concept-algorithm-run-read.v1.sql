select id, algorithm_kind, algorithm_id, algorithm_version, parameters_hash,
                    concept_revision_watermark, relation_revision_watermark, paper_scope_hash,
                    status, result_count, created_at, started_at, finished_at
             from concept_algorithm_runs where id = ?1
