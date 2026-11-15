use std::sync::{Arc, atomic::AtomicBool};

#[test]
fn cancellation_is_checked_before_pdf_decode_and_can_be_cleared_for_reuse() {
    let mut pipeline = docling_pdf::Pipeline::new().unwrap();
    pipeline.set_cancel_flag(Some(Arc::new(AtomicBool::new(true))));
    let error = pipeline
        .convert_outcome(b"not a PDF", None, "fixture")
        .unwrap_err();
    assert!(error.to_string().contains("cancelled"));
    pipeline.set_cancel_flag(None);
    let error = pipeline
        .convert_outcome(b"not a PDF", None, "fixture")
        .unwrap_err();
    assert!(!error.to_string().contains("cancelled"));
}
