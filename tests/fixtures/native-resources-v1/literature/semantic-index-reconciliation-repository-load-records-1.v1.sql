select r.id, r.representation_id, r.lance_row_id, sr.input_sha256, r.status
         from embedding_records r
         join semantic_representations sr on sr.id = r.representation_id
         where r.index_generation_id = ?1 and r.status != 'tombstoned'
         order by r.id