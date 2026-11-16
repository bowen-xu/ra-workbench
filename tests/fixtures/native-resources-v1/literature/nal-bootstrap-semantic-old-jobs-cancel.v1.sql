update embedding_outbox_jobs
         set status = 'canceled', error_code = 'superseded_representation',
             lease_owner = null, lease_expires_at = null,
             finished_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         where status in ('queued', 'running') and embedding_record_id in (
             select r.id from embedding_records r
             join semantic_representations sr on sr.id = r.representation_id
             where sr.subject_kind = 'concept' and sr.subject_id = ?1
               and sr.subject_revision < ?2
         )