update paper_analysis_turn_attempts
         set status = 'interrupted', error_code = 'interrupted',
             error_message = '应用上次退出时阶段仍在运行；不会自动重新发送论文',
             finished_at = coalesce(finished_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         where status = 'running';
         update paper_analysis_turns
         set status = 'interrupted',
             finished_at = coalesce(finished_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         where status = 'running';
         update paper_analysis_workflows
         set status = case
                 when active_turn_kind = 'paper_concept_map' and exists(
                     select 1 from paper_analysis_turns overview_turn
                     where overview_turn.workflow_id = paper_analysis_workflows.id
                       and overview_turn.turn_kind = 'paper_overview'
                       and overview_turn.status = 'succeeded'
                 ) then 'partial'
                 else 'failed'
             end,
             active_turn_kind = null,
             revision = revision + 1,
             completed_at = coalesce(completed_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
             updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         where status in ('preparing', 'overview_running', 'concepts_running');
