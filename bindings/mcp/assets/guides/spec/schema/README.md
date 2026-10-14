# Generated structural schemas

These Draft 7 schemas describe the current serialized Rust records for models,
run requests, results, expressions and regimens. They are generated from the
same serde types used by execution, with the optional `qspquest-core/schema`
feature and schemars 0.8.22. They describe the supported public document contracts.

Schema validation checks document shape, required fields, enums, expression
argument shapes and rejection of additional properties where the Rust type
rejects them. It does not establish scientific validity or runnable support.
Compilation and binding still validate units, symbol references, domains,
limits and supported capabilities. Execution validates tolerance modes and
rejects non-null seeds. Structurally valid documents can fail these checks.

An installed MCP client reads the schemas through the schema resource URIs in
`qspquest://documentation`; structural validation alone does not imply
execution support. The following source-development commands require the
release source, Rust toolchain and Python 3.10 or later. They regenerate and
verify without running a solver:

```sh
cargo run -p qspquest-core --features schema --example export_schemas -- spec/schema
cargo run -p qspquest-core --features schema --example export_schemas -- spec/schema --check
python scripts/generate_mcp_resources.py --check
python scripts/check_documentation.py
```

The generator writes files only in the example executable. Library callers can
obtain schema values from `qspquest_core::schema::documents()`. The feature is
absent from the default WASM build; schema tooling need not ship to browsers.
