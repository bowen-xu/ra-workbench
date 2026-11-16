select distinct relation_assertion_id from concept_identity_relation_impacts
         where event_id = ?1 order by relation_assertion_id
