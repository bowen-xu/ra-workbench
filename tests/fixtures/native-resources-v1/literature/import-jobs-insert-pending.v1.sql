insert into import_results
                (id, job_id, input_label, state, raw_input, created_at, updated_at)
             values
                (?1, ?2, ?3, 'pending', ?4, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))