insert into merge_operations
            (id, survivor_item_id, merged_item_id, snapshot_json, state, created_at)
         values (?1, ?2, ?3, ?4, 'applied', ?5)