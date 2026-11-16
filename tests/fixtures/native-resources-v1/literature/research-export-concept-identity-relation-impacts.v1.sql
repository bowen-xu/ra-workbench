select event_id, relation_assertion_id, endpoint_role, original_concept_id,
                     resolved_concept_id from concept_identity_relation_impacts
              order by event_id, relation_assertion_id, endpoint_role
