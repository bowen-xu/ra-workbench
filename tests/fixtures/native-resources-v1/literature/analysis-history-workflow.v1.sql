select workflow.id, workflow.paper_id, workflow.context_snapshot_id, status, provider_id, model_id,
                    reasoning_effort, prompt_set_version, active_turn_kind, revision,
                    supersedes_workflow_id, workflow.created_at, workflow.updated_at, completed_at,
                    binding.conversation_id
             from paper_analysis_workflows workflow
             left join agent_paper_analysis_bindings binding on binding.workflow_id = workflow.id
             where workflow.id = ?1