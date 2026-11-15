#[cfg(debug_assertions)]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    research_assistant_lib::run_native_preview_check()
}

#[cfg(not(debug_assertions))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    Err("native preview acceptance is development-only".into())
}
