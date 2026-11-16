update paper_analysis_workflows set supersedes_workflow_id=null where supersedes_workflow_id in
            (select p.id from paper_analysis_workflows p join literature_cleanup_rows s
             on s.item_id=?1 and s.table_name=?2 and s.row_key=p.rowid)