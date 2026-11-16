insert into embedding_reconciliation_runs (id, status, started_at)
         values (?1, 'running', strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))