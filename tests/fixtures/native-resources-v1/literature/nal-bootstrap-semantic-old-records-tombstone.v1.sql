update embedding_records
         set status = 'tombstoned', error_code = 'superseded_representation',
             stale_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
             updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         where status != 'tombstoned' and representation_id in (
             select id from semantic_representations
             where subject_kind = 'concept' and subject_id = ?1 and subject_revision < ?2
         )