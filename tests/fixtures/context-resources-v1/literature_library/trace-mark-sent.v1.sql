update concept_retrieval_traces
         set status = 'sent', sent_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         where id = ?1 and status = 'previewed'