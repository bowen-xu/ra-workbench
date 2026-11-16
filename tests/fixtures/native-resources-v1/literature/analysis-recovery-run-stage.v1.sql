update ai_run_stages
         set status = 'succeeded', output_hash = ?2, error_code = null, error_message = null,
             finished_at = coalesce(finished_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         where id = (
             select id from ai_run_stages
             where run_id = ?1 and stage_kind = 'validating'
               and status in ('running', 'failed')
             order by attempt desc limit 1
         )