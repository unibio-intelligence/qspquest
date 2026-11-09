//! Friendly doses into a concentration state whose dosing mapping has a
//! dimensional scale: `dosing c [mg] = SCALE` with SCALE in 1/L.
use qspquest_mcp::{library::Library, policy::Tier, server::Server};
use rmcp::{model::CallToolRequestParams, ServiceExt};
use serde_json::{json, Value};

fn source(scale: &str) -> String {
    format!(
        "model scale_probe
time_unit = h
parameters
  cl = 0.3 [L/h] log
  v = 3 [L] log
  inv_v = 0.333333333333 [1/L] log
states
  c = 0 [mg/L]
dynamics
  d/dt(c) = input_c - cl / v * c
outputs
  cp = c [mg/L]
dosing
  c [mg] = {scale}
end
"
    )
}

#[tokio::test]
async fn dimensional_dose_scales_accept_friendly_doses_in_the_declared_unit() {
    for (index, scale) in ["inv_v", "1 / v", "quantity(0.333333333333, \"1/L\")"]
        .into_iter()
        .enumerate()
    {
        // The model card is read from a one-model .qsp directory library.
        let dir = std::env::temp_dir().join(format!(
            "qspquest-dose-scales-{}-{index}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("scale_probe.qsp"), source(scale)).unwrap();
        let library = Library::directory(&dir).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        let (a, b) = tokio::io::duplex(1_000_000);
        let server = Server::new(library, false, Tier::Free, 2);
        let task =
            tokio::spawn(async move { server.serve(a).await.unwrap().waiting().await.unwrap() });
        let client = ().serve(b).await.unwrap();
        let call = |tool: &'static str, arguments: Value| {
            let client = &client;
            async move {
                client
                    .call_tool(
                        CallToolRequestParams::new(tool)
                            .with_arguments(arguments.as_object().unwrap().clone()),
                    )
                    .await
                    .unwrap()
                    .structured_content
                    .unwrap()
            }
        };
        let card = call("get_model", json!({"id": "scale_probe"})).await;
        let target = &card["interface"]["dose_targets"][0];
        assert_eq!(target["amount_unit"], "mg", "{scale}: {card}");
        assert!(target["dose_scale"].is_string(), "{scale}: {target}");
        let model = json!({"source": source(scale)});
        // 6 mg into 3 L, given in three compatible units, gives 2 mg/L just after the dose.
        for amount in [
            json!({"value":6,"unit":"mg"}),
            json!({"value":0.006,"unit":"g"}),
            json!({"value":6,"unit":"1"}),
        ] {
            let run = call(
                "simulate",
                json!({"model": model, "end":{"value":1,"unit":"h"}, "times":[{"value":0,"unit":"h"}],
                    "doses":[{"target":"c","amount":amount,"at":{"value":0,"unit":"h"}}],
                    "detail":"full","outputs":["cp"]}),
            )
            .await;
            assert!(run.get("error").is_none(), "{scale} {amount}: {run}");
            let sides = run["sides"].as_array().unwrap();
            let values = run["outputs"][0]["values"].as_array().unwrap();
            let after = sides
                .iter()
                .position(|side| side == "post")
                .map(|i| values[i].as_f64().unwrap())
                .unwrap();
            assert!((after - 2.0).abs() < 1e-6, "{scale} {amount}: {after}");
        }
        let refused = call(
            "simulate",
            json!({"model": model, "end":{"value":1,"unit":"h"},
                "doses":[{"target":"c","amount":{"value":2,"unit":"mg/L"},"at":{"value":0,"unit":"h"}}]}),
        )
        .await;
        let message = refused["error"]["message"].as_str().unwrap();
        assert!(message.contains("'mg'"), "{scale}: {message}");
        client.cancel().await.unwrap();
        task.abort();
    }
}
