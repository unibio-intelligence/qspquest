use qspquest_mcp::{
    library::{hash, Library},
    policy::{self, Tier},
    server::Server,
};
use rmcp::{
    model::{CallToolRequestParams, ReadResourceRequestParams},
    ServiceExt,
};
use serde_json::{json, Value};

#[tokio::test]
async fn model_cards_preserve_full_details_and_stored_scenarios() {
    let library = Library::embedded().unwrap();
    let expected = library.records.clone();
    let (a, b) = tokio::io::duplex(1_000_000);
    let task = tokio::spawn(async move {
        Server::new(library, false, Tier::Free, 2)
            .serve(a)
            .await
            .unwrap()
            .waiting()
            .await
            .unwrap()
    });
    let client = ().serve(b).await.unwrap();
    for record in expected {
        let result = client
            .call_tool(
                CallToolRequestParams::new("get_model").with_arguments(
                    json!({"id":record.metadata["id"], "version":record.metadata["version"]})
                        .as_object()
                        .unwrap()
                        .clone(),
                ),
            )
            .await
            .unwrap();
        assert_ne!(result.is_error, Some(true), "{result:?}");
        assert!(serde_json::to_vec(&result).unwrap().len() < policy::RESPONSE_LIMIT);
        let card = result.structured_content.unwrap();
        assert_eq!(card["detail"], "compact");
        assert_eq!(card["archive_sha256"], record.archive_hash);
        assert_eq!(card["time_unit"], record.model["time_unit"]);
        let output_count = record.model["outputs"].as_array().unwrap().len();
        assert_eq!(
            card["interface"]["output_contract"]["output_count"],
            output_count
        );
        assert_eq!(
            card["interface"]["output_contract"]["descriptors"]
                .as_array()
                .unwrap()
                .len(),
            output_count.min(32)
        );
        assert_eq!(
            card["interface"]["output_contract"]["descriptors_complete"],
            output_count <= 32
        );
        let name = record.model["outputs"][0]["name"].as_str().unwrap();
        let focused = client
            .call_tool(
                CallToolRequestParams::new("get_model").with_arguments(
                    json!({"id":record.metadata["id"], "outputs":[name]})
                        .as_object()
                        .unwrap()
                        .clone(),
                ),
            )
            .await
            .unwrap();
        assert_ne!(focused.is_error, Some(true));
        let focus = focused.structured_content.unwrap();
        assert_eq!(
            focus["interface"]["output_contract"]["descriptors"][0]["canonical_output"],
            name
        );
        assert_eq!(
            focus["interface"]["output_contract"]["descriptors"]
                .as_array()
                .unwrap()
                .len(),
            1
        );

        assert!(!card["interface"]["parameters"]
            .as_array()
            .unwrap()
            .is_empty());
        assert!(card.get("source").is_none() && card.get("native_execution").is_none());
        assert!(card.get("parameters").is_none() && card.get("equations").is_none());
        let response = client
            .read_resource(ReadResourceRequestParams::new(
                card["full_model_details_resource"].as_str().unwrap(),
            ))
            .await
            .unwrap();
        let mut text = serde_json::to_value(response).unwrap()["contents"][0]["text"]
            .as_str()
            .unwrap()
            .to_owned();
        let manifest: Value = serde_json::from_str(&text).unwrap();
        if manifest["kind"] == "utf8_chunk_manifest" {
            text.clear();
            for uri in manifest["chunks"].as_array().unwrap() {
                let response = client
                    .read_resource(ReadResourceRequestParams::new(uri.as_str().unwrap()))
                    .await
                    .unwrap();
                text.push_str(
                    serde_json::to_value(response).unwrap()["contents"][0]["text"]
                        .as_str()
                        .unwrap(),
                );
            }
            assert_eq!(hash(text.as_bytes()), manifest["sha256"]);
        }
        let full: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(full["detail"], "full");
        assert_eq!(full["metadata"], record.metadata);
        assert_eq!(full["parameters"], record.model["parameters"]);
        assert_eq!(full["outputs"], record.model["outputs"]);
        assert_eq!(full["native_execution"], record.index["native_execution"]);
        assert_eq!(full["archive_sha256"], card["archive_sha256"]);
        assert!(full.get("equations").is_some());
        for (sid, request) in record.scenarios {
            let result = client.call_tool(CallToolRequestParams::new("get_scenario_request").with_arguments(
                json!({"model_id":record.metadata["id"], "version":record.metadata["version"], "scenario_id":sid}).as_object().unwrap().clone()
            )).await.unwrap();
            assert_ne!(result.is_error, Some(true));
            assert_eq!(result.structured_content.unwrap()["request"], request);
        }
    }
    let result = client
        .call_tool(
            CallToolRequestParams::new("get_model").with_arguments(
                json!({"model_id":"qspquest_diffdose_2026_idr_benchmark","detail":"full"})
                    .as_object()
                    .unwrap()
                    .clone(),
            ),
        )
        .await
        .unwrap();
    assert_ne!(result.is_error, Some(true));
    assert!(result
        .structured_content
        .unwrap()
        .get("full_model_details_resource")
        .is_some());
    assert!(client
        .read_resource(ReadResourceRequestParams::new(
            "qspquest://models/qspquest_diffdose_2026_idr_benchmark/details/missing-version"
        ))
        .await
        .is_err());
    let invalid = client
        .call_tool(
            CallToolRequestParams::new("get_model").with_arguments(
                json!({"model_id":"qspquest_diffdose_2026_idr_benchmark","detail":"unbounded"})
                    .as_object()
                    .unwrap()
                    .clone(),
            ),
        )
        .await;
    assert!(invalid.is_err() || invalid.unwrap().is_error == Some(true));
    client.cancel().await.unwrap();
    task.abort();
}

#[tokio::test]
async fn oversized_metadata_is_lossless_through_bounded_utf8_chunks() {
    let mut library = Library::embedded().unwrap();
    library
        .records
        .retain(|r| r.metadata["id"] == "qspquest_diffdose_2026_idr_benchmark");
    let record = &mut library.records[0];
    record.metadata["synthetic_large_annotation"] = json!("λ漢字".repeat(75_000));
    let expected_metadata = record.metadata.clone();
    let version = record.metadata["version"].clone();
    let mut held = record.clone();
    held.metadata = json!({"id":"held_example","version":"1","execution_available":false,"execution_blocker":"Synthetic metadata-only record", "format":"metadata_only"});
    held.model = Value::Null;
    held.scenarios.clear();
    library.records.push(held);
    let (a, b) = tokio::io::duplex(1_000_000);
    let task = tokio::spawn(async move {
        Server::new(library, false, Tier::Free, 2)
            .serve(a)
            .await
            .unwrap()
            .waiting()
            .await
            .unwrap()
    });
    let client = ().serve(b).await.unwrap();
    for detail in ["compact", "full"] {
        let result = client
            .call_tool(
                CallToolRequestParams::new("get_model").with_arguments(
                    json!({"model_id":"qspquest_diffdose_2026_idr_benchmark","detail":detail})
                        .as_object()
                        .unwrap()
                        .clone(),
                ),
            )
            .await
            .unwrap();
        assert_ne!(result.is_error, Some(true));
        assert!(serde_json::to_vec(&result).unwrap().len() < policy::RESPONSE_LIMIT);
        let card = result.structured_content.unwrap();
        assert_eq!(
            card["detail"],
            if detail == "full" {
                "full_resource"
            } else {
                "compact"
            }
        );
        assert!(card["metadata"].get("synthetic_large_annotation").is_none());
        let uri = card["full_model_details_resource"].as_str().unwrap();
        let response = client
            .read_resource(ReadResourceRequestParams::new(uri))
            .await
            .unwrap();
        assert!(serde_json::to_vec(&response).unwrap().len() < policy::RESPONSE_LIMIT);
        let manifest: Value = serde_json::from_str(
            serde_json::to_value(response).unwrap()["contents"][0]["text"]
                .as_str()
                .unwrap(),
        )
        .unwrap();
        assert_eq!(manifest["kind"], "utf8_chunk_manifest");
        let mut text = String::new();
        for uri in manifest["chunks"].as_array().unwrap() {
            let response = client
                .read_resource(ReadResourceRequestParams::new(uri.as_str().unwrap()))
                .await
                .unwrap();
            assert!(serde_json::to_vec(&response).unwrap().len() < policy::RESPONSE_LIMIT);
            text.push_str(
                serde_json::to_value(response).unwrap()["contents"][0]["text"]
                    .as_str()
                    .unwrap(),
            );
        }
        assert_eq!(hash(text.as_bytes()), manifest["sha256"]);
        let full: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(full["metadata"], expected_metadata);
        assert_eq!(full["metadata"]["version"], version);
        let out_of_range = format!("{uri}/chunks/999999");
        assert!(client
            .read_resource(ReadResourceRequestParams::new(out_of_range))
            .await
            .is_err());
    }
    let result = client
        .call_tool(
            CallToolRequestParams::new("get_model").with_arguments(
                json!({"model_id":"held_example"})
                    .as_object()
                    .unwrap()
                    .clone(),
            ),
        )
        .await
        .unwrap();
    let card = result.structured_content.unwrap();
    assert_eq!(card["execution_available"], false);
    assert_eq!(
        card["metadata"]["execution_blocker"],
        "Synthetic metadata-only record"
    );
    client.cancel().await.unwrap();
    task.abort();
}
