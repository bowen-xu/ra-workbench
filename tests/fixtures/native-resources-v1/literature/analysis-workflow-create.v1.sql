insert into paper_analysis_workflows
         (id, paper_id, context_snapshot_id, status, provider_id, model_id,
          reasoning_effort, prompt_set_version, supersedes_workflow_id, created_at, updated_at)
         values (?1, ?2, ?3, 'preparing', ?4, ?5, ?6, ?7, ?8,
                 strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
                 strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))