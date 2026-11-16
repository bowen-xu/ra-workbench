select id, revision, subject_id, subject_label_snapshot, predicate_kind,
                object_id, object_label_snapshot
         from relation_assertions
         where subject_kind = 'concept' and object_kind = 'concept'
           and (subject_id in (select value from json_each(?1))
                or object_id in (select value from json_each(?1)))
         order by updated_at desc, id
         limit ?2