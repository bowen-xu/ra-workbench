select id, 'subject', subject_id from relation_assertions
             where subject_kind = 'concept' and subject_id = ?1
             union all
             select id, 'object', object_id from relation_assertions
             where object_kind = 'concept' and object_id = ?1
