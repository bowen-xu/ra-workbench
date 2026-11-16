select exists(select 1 from paper_analysis_workflows where paper_id=?1
         and status in ('preparing','overview_running','concepts_running'))