update reference_records set usage_anchor_id=null,semantic_state='unreviewed',usage_kind=null,usage_summary=''
        where usage_anchor_id in(select p.id from source_anchors p join literature_cleanup_rows s
         on s.item_id=?1 and s.table_name='source_anchors' and s.row_key=p.rowid)