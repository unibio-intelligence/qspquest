use qspquest_mcp::library::Library;
use std::path::Path;

/// The synthetic model library shipped with the tests.
pub fn library() -> Library {
    Library::directory(Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/library"
    )))
    .unwrap()
}
