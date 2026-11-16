update concept_evolution_results set evidence_anchor_id=null where evidence_anchor_id in
            (select p.id from source_anchors p join literature_cleanup_rows s
             on s.item_id=?1 and s.table_name=?2 and s.row_key=p.rowid)