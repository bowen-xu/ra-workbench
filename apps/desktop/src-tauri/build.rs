fn main() {
    // Directory inputs must retrigger resource discovery after the native helper
    // is first generated or replaced by prepare:docling.
    println!("cargo:rerun-if-changed=.generated/docling");
    tauri_build::build();
}
