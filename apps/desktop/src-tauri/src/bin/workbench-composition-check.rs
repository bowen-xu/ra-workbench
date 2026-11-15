#[cfg(debug_assertions)]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    research_assistant_lib::run_workbench_composition_check()
}

#[cfg(not(debug_assertions))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    Err("workbench composition acceptance is development-only".into())
}
