use qspquest_core::fit::{FitProblem, FitRequest};
use qspquest_modeling::{document::sensitivity::CompiledSensitivityDocument, language};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 3 {
        return Err("usage: profiled_proportional_fit MODEL.qsp FIT.json".into());
    }
    let model = language::parse(&std::fs::read_to_string(&args[1])?)
        .map_err(|diagnostic| std::io::Error::other(format!("{diagnostic:?}")))?
        .document;
    let request: FitRequest = serde_json::from_str(&std::fs::read_to_string(&args[2])?)?;
    if !matches!(&request.problem, FitProblem::ProfiledProportional(_)) {
        return Err("example requires problem.kind=profiled_proportional".into());
    }
    let compiled = CompiledSensitivityDocument::compile(&model, request.parameters())?;
    println!(
        "{}",
        serde_json::to_string_pretty(&compiled.execute_fit(&request)?)?
    );
    Ok(())
}
