update metadata_provenance set source_reference = ?2
             where item_id = ?1 and source_kind = 'browser_connector'
