//! Plan the shipped synthetic objective; --execute explicitly evaluates it.
use qspquest_core::{
    fit::{FitProblem, FitRequest},
    model::ModelDocument,
};
use qspquest_modeling::document::sensitivity::CompiledSensitivityDocument;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if !args.is_empty() && args != ["--execute"] {
        return Err("usage: plan_gaussian_objective [--execute]".into());
    }
    let document: ModelDocument = serde_json::from_str(include_str!(
        "../../../conformance/models/synthetic-one-compartment.json"
    ))?;
    let request: FitRequest = serde_json::from_str(include_str!(
        "../../../conformance/requests/synthetic-fit.json"
    ))?;
    let FitProblem::Individual(fit) = request.problem else {
        return Err("the example requires the shipped individual fixture".into());
    };
    let model =
        CompiledSensitivityDocument::compile(&document, &fit.objective.simulation.with_respect_to)?;
    let output = if args.is_empty() {
        serde_json::to_value(model.plan_gaussian_objective(&fit.objective)?)?
    } else {
        serde_json::to_value(model.gaussian_objective(&fit.objective)?)?
    };
    println!("{}", serde_json::to_string_pretty(&output)?);
    Ok(())
}
