use super::*;
use std::sync::{Arc, Mutex};

#[test]
fn actual_term_and_concept_readbacks_keep_metadata_and_preferred_reread_order() {
    let fixture = Fixture::new();
    fixture.term("preferred", "preferred", "Preferred");
    fixture.term("alias", "alias", "Alias");
    fixture.concept("concept", "Actual label");
    let db = fixture.db.lock().unwrap();
    db.execute("insert into concept_terms(concept_id,term_id,relation_kind,created_at) values('concept','preferred','preferred','a'),('concept','alias','alias','b')",[]).unwrap();
    let seen = Arc::new(Mutex::new(Vec::<String>::new()));
    let observed = Arc::clone(&seen);
    db.create_scalar_function(
        "test_term_read",
        1,
        rusqlite::functions::FunctionFlags::SQLITE_UTF8,
        move |context| {
            let value = context.get::<String>(0)?;
            observed.lock().unwrap().push(value.clone());
            Ok(value)
        },
    )
    .unwrap();
    db.execute_batch("create temp view terms as select id,test_term_read(text) as text,normalized_text,language,provenance_kind,revision,created_at,updated_at from main.terms;").unwrap();
    let concept = fixture.resources.read_concept_in(&db, "concept").unwrap();
    assert_eq!(*seen.lock().unwrap(), ["Preferred", "Alias", "Preferred"]);
    assert_eq!(concept.preferred_term.created_at, "term-created");
    assert_eq!(concept.preferred_term.updated_at, "term-updated");
    assert_eq!(concept.created_at, "concept-created");
    assert_eq!(concept.updated_at, "concept-updated");
    assert_eq!(concept.terms.len(), 2);
    assert_eq!(concept.definition_count, 0);
    assert_eq!(concept.source_count, 0);
    let snapshot = serde_json::to_value(&concept).unwrap();
    assert_eq!(snapshot.as_object().unwrap().len(), 15);
    assert_eq!(snapshot["preferredTerm"].as_object().unwrap().len(), 8);
    db.execute_batch("drop view temp.terms;").unwrap();
    assert!(matches!(
        fixture.resources.read_term_in(&db, "absent"),
        Err(CaptureError::Shape("missing-term"))
    ));
}

#[test]
fn missing_preferred_mapping_precedes_a_later_unreadable_alias_term() {
    let fixture = Fixture::new();
    fixture.term("alias", "alias", "Alias");
    fixture.concept("concept", "Actual label");
    let db = fixture.db.lock().unwrap();
    db.execute("insert into concept_terms(concept_id,term_id,relation_kind,created_at) values('concept','alias','alias','a')",[]).unwrap();
    db.execute(
        "update terms set text=cast(x'FF' as text) where id='alias'",
        [],
    )
    .unwrap();
    assert!(matches!(
        fixture.resources.read_concept_in(&db, "concept"),
        Err(CaptureError::Shape("missing-preferred-term"))
    ));
}
