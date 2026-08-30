//! Compiles the canonical schemas at src/protos/ (repo root) with a pure-Rust
//! toolchain: protox parses/links, prost-build generates types. No system
//! `protoc` anywhere (ADR-0003).

fn main() -> Result<(), Box<dyn std::error::Error>> {
    const PROTOS: &[&str] = &[
        "../../src/protos/mcw.proto",
        "../../src/protos/journal.proto",
    ];
    const INCLUDES: &[&str] = &["../../src/protos"];

    for proto in PROTOS {
        println!("cargo:rerun-if-changed={proto}");
    }

    let file_descriptors = protox::compile(PROTOS, INCLUDES)?;
    prost_build::Config::new().compile_fds(file_descriptors)?;
    Ok(())
}
