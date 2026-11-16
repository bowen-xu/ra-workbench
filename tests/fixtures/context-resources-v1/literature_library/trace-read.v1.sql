select paper_id, query_sha256, status, context_text, context_sha256,
                    expires_at <= strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
             from concept_retrieval_traces where id = ?1