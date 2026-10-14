//! Export numerical qualification inputs/results to an explicit private evidence directory.
use qspquest_mcp::{core::compute, library::Library};
use serde_json::{json, Value};
fn main() {
    let args: Vec<_> = std::env::args().collect();
    let directory = std::path::PathBuf::from(args.get(1).expect("explicit evidence directory"));
    std::fs::create_dir_all(&directory).unwrap();
    let large = args.iter().any(|s| s == "--large");
    let wasm_requested = args.iter().any(|s| s == "--wasm");
    let refinement: i32 = args
        .iter()
        .find_map(|s| s.strip_prefix("--refinement=").map(|v| v.parse().unwrap()))
        .unwrap_or(0);
    assert!(
        (0..=2).contains(&refinement),
        "refinement must be 0, 1 or 2"
    );
    let filter = args.iter().find_map(|s| s.strip_prefix("--case="));
    let old_tight = args.get(2).is_some_and(|s| s == "tight");
    #[cfg(feature = "sandbox")]
    let sandbox = wasm_requested.then(|| qspquest_mcp::sandbox::Sandbox::embedded().unwrap());
    #[cfg(not(feature = "sandbox"))]
    assert!(!wasm_requested, "--wasm needs the sandbox feature");
    let library = Library::embedded().unwrap();
    for record in library.records {
        let model = record.metadata["id"].as_str().unwrap();
        if large && !model.contains("qspquest_liu_") && !model.contains("qspquest_rajwade_") {
            continue;
        }
        if old_tight && !model.contains("bite_benchmark") && !model.contains("qspquest_yuan_") {
            continue;
        }
        let document = serde_json::from_value(record.model.clone()).unwrap();
        let compiled = qspquest_modeling::document::CompiledDocument::compile(&document).unwrap();
        for (id, mut request) in record.scenarios {
            if filter.is_some_and(|f| !id.contains(f)) {
                continue;
            }
            if old_tight {
                request["regimen"]["rtol"] = json!(1e-11);
                request["regimen"]["atol"] = json!(1e-13);
            }
            let factor = 10f64.powi(-refinement);
            if refinement > 0 {
                request["regimen"]["rtol"] =
                    json!(request["regimen"]["rtol"].as_f64().unwrap() * factor);
                if let Some(a) = request["regimen"]["atol"].as_f64() {
                    request["regimen"]["atol"] = json!(a * factor);
                }
                if let Some(values) = request["regimen"]["absolute_tolerances"].as_object_mut() {
                    for quantity in values.values_mut() {
                        quantity["value"] = json!(quantity["value"].as_f64().unwrap() * factor);
                    }
                }
            }
            if args.iter().any(|s| s == "--dose-sides") {
                let administrations = request["regimen"]["administrations"]
                    .as_array()
                    .unwrap()
                    .clone();
                let begin = request["regimen"]["start"]["value"].as_f64().unwrap();
                let end = request["regimen"]["end"]["value"].as_f64().unwrap();
                let observations = request["regimen"]["observations"].as_array_mut().unwrap();
                for dose in administrations {
                    let time = dose["time"].clone();
                    if !(begin..=end).contains(&time["value"].as_f64().unwrap()) {
                        continue;
                    }
                    for side in ["pre", "post"] {
                        if !observations
                            .iter()
                            .any(|o| o["time"] == time && o["side"] == side)
                        {
                            observations.push(json!({"time":time,"side":side}));
                        }
                    }
                }
                observations.sort_by(|a, b| {
                    a["time"]["value"]
                        .as_f64()
                        .unwrap()
                        .total_cmp(&b["time"]["value"].as_f64().unwrap())
                        .then_with(|| {
                            let rank = |v: &Value| if v["side"] == "pre" { 0 } else { 1 };
                            rank(a).cmp(&rank(b))
                        })
                });
            }
            let start = std::time::Instant::now();
            let native =
                compute::execute_compiled(&compiled, &document, "simulate", request.clone());
            let native_ms = start.elapsed().as_secs_f64() * 1000.;
            let mut data = json!({"model":record.model,"request":request,"refinement":refinement,"native_ms":native_ms});
            match native {
                Ok(r) => data["result"] = r,
                Err(e) => data["native_error"] = e,
            };
            #[cfg(feature = "sandbox")]
            if let Some(sandbox) = &sandbox {
                let start = std::time::Instant::now();
                match sandbox.profile_library(record.model.clone(), "simulate", request.clone()) {
                    Ok((r, fuel)) => {
                        data["wasm_result"] = r;
                        data["wasm_fuel"] = json!(fuel);
                    }
                    Err(e) => data["wasm_error"] = e,
                }
                data["wasm_ms"] = json!(start.elapsed().as_secs_f64() * 1000.);
            }
            // Explicit errors are retained instead of turning an incomplete sweep into a pass.
            if data.get("result").is_none() {
                data["result"] = Value::Null;
            }
            std::fs::write(
                directory.join(format!("{model}--{id}.json")),
                serde_json::to_vec(&data).unwrap(),
            )
            .unwrap();
            eprintln!("{model} {id} refinement={refinement} native={native_ms:.2}ms");
        }
    }
}
