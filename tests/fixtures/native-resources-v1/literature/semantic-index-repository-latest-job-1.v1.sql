select j.id, sr.id, sr.subject_kind, sr.subject_id, j.status, j.attempt,
                    j.error_code, j.created_at, j.finished_at
             from embedding_outbox_jobs j
             join embedding_records r on r.id = j.embedding_record_id
             join semantic_representations sr on sr.id = r.representation_id
             where j.embedding_record_id = ?1
             order by j.attempt desc limit 1