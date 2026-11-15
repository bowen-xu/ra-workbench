fn main() {
    protobuf_codegen_pure::Codegen::new()
        .out_dir(std::env::var("OUT_DIR").expect("Cargo supplies OUT_DIR"))
        .inputs(["proto/log.proto"])
        .include("proto")
        .customize(protobuf_codegen_pure::Customize {
            gen_mod_rs: Some(true),
            ..Default::default()
        })
        .run()
        .expect("checked-in log schema compiles");
    println!("cargo:rerun-if-changed=proto/log.proto");
}
