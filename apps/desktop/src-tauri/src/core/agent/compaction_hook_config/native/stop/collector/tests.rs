use super::*;

#[test]
fn rejects_duplicate_identities_and_invalid_transitions() {
    let id = Uuid::new_v4();
    let run = Uuid::new_v4();
    assert!(StopHandlerCollector::new(vec![(id, run), (id, Uuid::new_v4())]).is_none());
    assert!(StopHandlerCollector::new(vec![(id, run), (Uuid::new_v4(), run)]).is_none());
    let collector = StopHandlerCollector::new(vec![(id, run)]).unwrap();
    collector.notify(id, run, StopHandlerNotification::Canceled);
    assert!(collector.outcomes(true).is_none());
    let collector = StopHandlerCollector::new(vec![(id, run)]).unwrap();
    collector.notify(Uuid::new_v4(), run, StopHandlerNotification::Started);
    assert!(collector.outcomes(true).is_none());
}

#[test]
fn only_cancellation_allows_unpolled_admission() {
    let id = Uuid::new_v4();
    let run = Uuid::new_v4();
    let collector = StopHandlerCollector::new(vec![(id, run)]).unwrap();
    assert!(collector.outcomes(false).is_none());
    assert_eq!(
        collector.outcomes(true),
        Some(vec![(id, run, StopCollectedOutcome::NotStarted)])
    );
    collector.notify(id, run, StopHandlerNotification::Started);
    assert!(collector.outcomes(true).is_none());
    collector.notify(id, run, StopHandlerNotification::Canceled);
    assert_eq!(
        collector.outcomes(true),
        Some(vec![(id, run, StopCollectedOutcome::Canceled)])
    );
}
