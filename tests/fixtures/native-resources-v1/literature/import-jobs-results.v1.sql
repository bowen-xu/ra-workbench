select input_label, coalesce(raw_input, ''), state, item_id, error_message
             from import_results where job_id = ?1 order by created_at, rowid