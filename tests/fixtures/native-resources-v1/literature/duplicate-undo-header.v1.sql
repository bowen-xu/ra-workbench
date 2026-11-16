select survivor_item_id, merged_item_id, snapshot_json from merge_operations
             where id = ?1 and state = 'applied'