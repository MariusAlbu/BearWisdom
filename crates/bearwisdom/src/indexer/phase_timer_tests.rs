use super::*;

#[test]
fn scope_records_total_and_call_count() {
    // Two scopes on the same name sum into one entry with calls=2.
    {
        let _t = scope("test_phase_alpha");
    }
    {
        let _t = scope("test_phase_alpha");
    }
    let table = TABLE.lock().expect("table");
    let entry = table
        .iter()
        .find(|e| e.name == "test_phase_alpha")
        .expect("entry recorded");
    assert_eq!(entry.calls.load(Ordering::Relaxed), 2);
}

#[test]
fn record_creates_distinct_entries_per_name() {
    record("test_phase_beta", 100);
    record("test_phase_gamma", 200);
    let table = TABLE.lock().expect("table");
    let beta = table.iter().find(|e| e.name == "test_phase_beta").unwrap();
    let gamma = table.iter().find(|e| e.name == "test_phase_gamma").unwrap();
    assert_eq!(beta.total_ns.load(Ordering::Relaxed), 100);
    assert_eq!(gamma.total_ns.load(Ordering::Relaxed), 200);
}
