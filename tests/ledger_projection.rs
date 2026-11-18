use ascout::control_plane::{
    ledger::Ledger, observation::ObservationContent, ui_projection::UiProjection,
};

fn status() -> ObservationContent {
    ObservationContent::RunStatus {
        connected: true,
        ready: false,
        running: false,
        notice: "saved".into(),
    }
}

#[test]
fn failed_append_does_not_publish_a_memory_only_observation() {
    let mut ledger = Ledger::open(std::path::Path::new("/dev/full")).unwrap();
    assert!(ledger.append_observation("run-test", status()).is_err());
    assert_eq!(ledger.reader().sequence(), 0);
    assert!(ledger.reader().read_saved_after(0).unwrap().is_empty());
}

#[test]
fn projection_rebuilds_from_saved_records_and_reopen_keeps_sequence() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("events.jsonl");
    let mut ledger = Ledger::open(&path).unwrap();
    ledger.append_observation("run-test", status()).unwrap();
    let projection = UiProjection::new(ledger.reader());
    assert_eq!(projection.state_event_set().events.len(), 1);
    projection.discard_cache();
    assert_eq!(projection.state_event_set().events.len(), 1);
    drop(projection);
    drop(ledger);
    let mut ledger = Ledger::open(&path).unwrap();
    ledger.append_observation("run-test", status()).unwrap();
    let rebuilt = UiProjection::new(ledger.reader()).state_event_set();
    assert_eq!(rebuilt.sequence, 2);
    assert_eq!(rebuilt.events.len(), 2);
}
