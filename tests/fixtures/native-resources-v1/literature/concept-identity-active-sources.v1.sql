select source_concept_id from concept_identity_mappings
         where merge_event_id = ?1 order by source_concept_id
