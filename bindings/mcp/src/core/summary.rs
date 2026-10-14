use serde_json::{json, Value};
pub fn sampled_summary(result: &Value, selected: &Value) -> Value {
    let Some(times) = result["times"].as_array() else {
        return json!([]);
    };
    let n = times.len();
    if n == 0 {
        return json!([]);
    }
    let t: Vec<_> = times.iter().map(|v| v.as_f64().unwrap_or(0.0)).collect();
    let sides = result["sides"].as_array().cloned().unwrap_or_default();
    let mut summary = Vec::new();
    let time_unit = result["time_unit"].as_str().unwrap_or("").to_owned();
    if let Some(outputs) = result["outputs"].as_array() {
        for o in outputs.iter().filter(|o| {
            selected
                .as_array()
                .is_none_or(|a| a.iter().any(|name| *name == o["name"]))
        }) {
            let y: Vec<_> = o["values"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_f64().unwrap())
                .collect();
            let peak = (0..n).max_by(|a, b| y[*a].total_cmp(&y[*b])).unwrap();
            let min = y.iter().copied().fold(f64::INFINITY, f64::min);
            let auc = (1..n)
                .map(|i| (t[i] - t[i - 1]) * (y[i] + y[i - 1]) * 0.5)
                .sum::<f64>();
            let troughs: Vec<_> = (0..n)
                .filter(|i| sides.get(*i).is_some_and(|s| s == "pre"))
                .map(|i| json!({"time":t[i],"value":y[i]}))
                .collect();
            summary.push(json!({"output":o["name"],"unit":o["unit"],"sampled_max":y[peak],"sampled_tmax":t[peak],"sampled_min":min,"trapezoidal_auc":auc,"auc_unit":format!("{}.{}",o["unit"].as_str().unwrap_or(""),time_unit),"pre_event_samples":troughs.into_iter().take(200).collect::<Vec<_>>(),"interpretation":"Sampled extrema and grid-based trapezoidal integral; no extrapolated AUC or certified NCA. Pre-side samples are not necessarily dosing troughs."}));
        }
    }
    json!(summary)
}

pub fn compact(mut result: Value, selected: &Value) -> Value {
    let Some(times) = result["times"].as_array().cloned() else {
        return result;
    };
    let n = times.len();
    if n == 0 {
        return result;
    }
    let summary = sampled_summary(&result, selected);
    let t: Vec<_> = times.iter().map(|v| v.as_f64().unwrap_or(0.0)).collect();
    let sides = result["sides"].as_array().cloned().unwrap_or_default();
    let mut indices: std::collections::BTreeSet<usize> = (0..n.min(200))
        .map(|i| if n <= 200 { i } else { i * (n - 1) / 199 })
        .collect();
    for i in 1..n {
        if t[i] == t[i - 1] {
            indices.insert(i - 1);
            indices.insert(i);
        }
    }
    let keep: Vec<_> = indices.into_iter().collect();
    result["times"] = json!(keep.iter().map(|i| times[*i].clone()).collect::<Vec<_>>());
    result["sides"] = json!(keep.iter().map(|i| sides[*i].clone()).collect::<Vec<_>>());
    if let Some(outputs) = result["outputs"].as_array_mut() {
        outputs.retain(|o| {
            selected
                .as_array()
                .is_none_or(|a| a.iter().any(|name| *name == o["name"]))
        });
        for o in outputs {
            let v = o["values"].as_array().unwrap();
            o["values"] = json!(keep.iter().map(|i| v[*i].clone()).collect::<Vec<_>>())
        }
    }
    result["summary"] = summary;
    result["trajectory_kind"] = json!("compact_preview");
    result["trajectory_notice"] = json!("Compact preview on selected sample rows. Retrieve full_result_resource for the complete requested trajectory; do not treat preview rows as the requested observation grid.");
    result["sampling"] =
        json!({"original_points":n,"returned_points":keep.len(),"event_sides_preserved":true});
    result
}
