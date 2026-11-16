select json_extract(config_json, '$.semanticStatus'), embedding_space_id,
                    index_generation_id, source_count, external_paper_count,
                    context_sha256, expires_at
             from concept_retrieval_traces where id = ?1