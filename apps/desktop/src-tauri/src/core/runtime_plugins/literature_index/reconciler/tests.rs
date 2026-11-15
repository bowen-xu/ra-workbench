use super::*;
use crate::semantic_engine::StoredVectorMetadata;

#[test]
fn only_the_observed_physical_receipt_can_acknowledge_a_reconciled_row() {
    let stored = StoredVectorMetadata {
        row_id: "row".into(),
        input_sha256: "expected".into(),
        dataset_version: 17,
        vector_norm: -0.0,
    };
    assert!(confirm_reconciled_receipt(Some(&stored), "expected", 17, (-0.0f32).to_bits()).is_ok());
    for (hash, version, norm) in [
        ("different", 17, (-0.0f32).to_bits()),
        ("expected", 18, (-0.0f32).to_bits()),
        ("expected", 17, 0.0f32.to_bits()),
    ] {
        assert!(confirm_reconciled_receipt(Some(&stored), hash, version, norm).is_err());
    }
    assert!(confirm_reconciled_receipt(None, "expected", 17, (-0.0f32).to_bits()).is_err());
}

#[test]
fn physical_receipt_binding_keeps_exact_nan_payload_and_u64_version() {
    let stored = StoredVectorMetadata {
        row_id: "row".into(),
        input_sha256: "expected".into(),
        dataset_version: u64::MAX,
        vector_norm: f32::from_bits(0x7fc0_1234),
    };
    assert!(confirm_reconciled_receipt(Some(&stored), "expected", u64::MAX, 0x7fc0_1234).is_ok());
    assert!(confirm_reconciled_receipt(Some(&stored), "expected", u64::MAX, 0x7fc0_1235).is_err());
}
