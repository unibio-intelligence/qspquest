fn main() {
    println!(
        "cargo:rustc-env=QSPQUEST_MCP_TARGET={}",
        std::env::var("TARGET").unwrap()
    );
    println!("cargo:rerun-if-env-changed=QSPQUEST_SANDBOX_WASM");
    if std::env::var_os("CARGO_FEATURE_SANDBOX").is_some() {
        let path = std::env::var("QSPQUEST_SANDBOX_WASM").unwrap_or_else(|_| {
            let root = std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap())
                .join("../../target");
            let preserved = root.join("mcp-assets/sandbox.wasm");
            if preserved.is_file() {
                preserved.to_string_lossy().into_owned()
            } else {
                root.join("wasm32-unknown-unknown/release/qspquest_wasm.wasm")
                    .to_string_lossy()
                    .into_owned()
            }
        });
        let bytes =
            std::fs::read(&path).expect("Build the sandbox ABI first: scripts/build-mcp.sh");
        assert!(bytes.starts_with(b"\0asm"), "Sandbox must be a WASM module");
        println!("cargo:rerun-if-changed={path}");
        std::fs::write(
            std::path::Path::new(&std::env::var("OUT_DIR").unwrap()).join("sandbox.wasm"),
            bytes,
        )
        .unwrap();
    }
    if std::env::var_os("CARGO_FEATURE_WIDGET").is_some() {
        let dir = std::env::var("QSPQUEST_BROWSER_ASSETS_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|_| {
                std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap())
                    .join("../wasm/pkg/engine")
            });
        let out = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap());
        println!("cargo:rerun-if-env-changed=QSPQUEST_BROWSER_ASSETS_DIR");
        for (name, target) in [
            ("qspquest_wasm.js", "browser-engine.mjs"),
            ("qspquest_wasm_bg.wasm", "browser-engine.wasm"),
        ] {
            let path = dir.join(name);
            println!("cargo:rerun-if-changed={}", path.display());
            std::fs::copy(path, out.join(target)).expect(
                "Build current browser bindings before enabling widget: scripts/build-mcp.sh",
            );
        }
    }
}
