use super::Fixture;

#[test]
fn concept_trigger_changes_are_visible_only_to_later_original_source_batches() {
    let f = Fixture::new();
    f.data.db.lock().unwrap().execute_batch("insert into collections(id,name,description) values('a','First collection',''),('b','Second collection','');
        create trigger merge_later_sources after insert on concepts when new.label='原归类' begin
        update collections set name='Changed after capture' where id='a';
        insert into collections(id,name,description) values('not-captured','Not captured','');
        insert into collection_relation_types(id,name,inverse_name,directed,built_in) values('late-type','Late type','Late inverse',0,0);
        insert into collection_relations(id,source_id,target_id,type_id) values('late-relation','a','b','late-type');
        insert into collection_memberships(item_id,collection_id) values('source','a'),('source','b');
        insert into item_relation_assignments(item_id,relation_id) values('source','late-relation'); end;").unwrap();
    f.apply().unwrap();
    for label in ["First collection", "Second collection", "Late type"] {
        assert_eq!(
            f.data
                .db
                .lock()
                .unwrap()
                .query_row(
                    "select count(*) from concepts where label=?1",
                    [label],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
            1
        );
    }
    assert_eq!(
        f.scalar::<i64>(
            "select count(*) from concepts where label in ('Changed after capture','Not captured')"
        ),
        0
    );
    assert_eq!(
        f.scalar::<String>("select name from collections where id='a'"),
        "Changed after capture"
    );
    assert_eq!(f.scalar::<i64>("select count(*) from collection_memberships where item_id='survivor' and collection_id='b'"),1);
    assert_eq!(f.scalar::<i64>("select count(*) from item_relation_assignments where item_id='survivor' and relation_id='late-relation'"),1);
    assert_eq!(f.scalar::<i64>("select count(*) from nal_statements"), 8);
    assert_eq!(f.scalar::<i64>("select count(*) from nal_statements where predicate=json_object('kind','concept','id',(select id from concepts where label='Late type'))"),2);
}

#[test]
fn term_insert_trigger_name_occupancy_is_resolved_at_the_original_later_point() {
    let f = Fixture::new();
    f.collection();
    f.data.db.lock().unwrap().execute_batch("create trigger merge_name_occupancy after insert on terms when new.text='Original' begin
        insert into concepts(id,label,explanation,provenance_kind,revision,created_at,updated_at)
        values('occupied-during-term','Original','Synthetic existing identity','human_original',1,'2026','2026');
        insert into concept_terms(concept_id,term_id,relation_kind,created_at)
        values('occupied-during-term',new.id,'preferred','2026'); end;").unwrap();
    f.apply().unwrap();
    assert_eq!(
        f.scalar::<String>("select id from concepts where label='Original'"),
        "occupied-during-term"
    );
    assert_eq!(
        f.scalar::<i64>("select count(*) from concepts where label='Original`旧归类-2'"),
        1
    );
    assert_eq!(
        f.scalar::<i64>("select count(*) from terms where text='Original'"),
        1
    );
    assert_eq!(f.scalar::<i64>("select count(*) from concept_terms where term_id=(select id from terms where text='Original') and relation_kind='preferred'"),2);
}
