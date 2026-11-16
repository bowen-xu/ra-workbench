select j.id, r.id, sr.namespace, g.relative_database_path, s.dimension,
                    r.lance_row_id, sr.id, sr.subject_kind, sr.subject_id,
                    sr.subject_revision, sr.representation_kind, sr.language,
                    sr.input_sha256, s.id, g.generation, sr.created_at,
                    s.provider_id, s.model_id, sr.rendered_text
             from embedding_outbox_jobs j
             join embedding_records r on r.id = j.embedding_record_id
             join semantic_representations sr on sr.id = r.representation_id
             join embedding_spaces s on s.id = r.embedding_space_id
             join embedding_index_generations g on g.id = r.index_generation_id
             where j.id = ?1