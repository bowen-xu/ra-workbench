fn main() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        // ONNX Runtime's Objective-C availability checks need Apple's compiler-rt.
        // rustc does not automatically add this archive when linking native ORT.
        let output = std::process::Command::new("xcrun")
            .args(["clang", "-print-file-name=libclang_rt.osx.a"])
            .output()?;
        if !output.status.success() {
            return Err("cannot locate Apple compiler runtime".into());
        }
        let path = std::path::PathBuf::from(String::from_utf8(output.stdout)?.trim());
        if !path.is_file() {
            return Err("Apple compiler runtime archive is missing".into());
        }
        let directory = path.parent().ok_or("compiler runtime path has no parent")?;
        println!("cargo:rustc-link-search=native={}", directory.display());
        println!("cargo:rustc-link-lib=static=clang_rt.osx");
    }
    Ok(())
}
