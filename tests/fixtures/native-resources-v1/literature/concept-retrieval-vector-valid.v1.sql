select exists(
                select 1 from embedding_records r
                join semantic_representations sr on sr.id = r.representation_id
                where r.index_generation_id = ?1 and r.status = 'ready'
                  and r.lance_row_id = ?2 and sr.id = ?3
                  and sr.subject_kind = 'concept' and sr.subject_id = ?4
                  and sr.subject_revision = ?5 and sr.representation_kind = 'concept_identity'
             )