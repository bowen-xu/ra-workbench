delete from concept_identity_mappings
             where source_concept_id = ?1 and merge_event_id = ?2
               and canonical_concept_id = ?3
