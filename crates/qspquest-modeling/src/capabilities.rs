//! Enforce minimum model requirements before compilation or binding.
use qspquest_core::{
    model::{EventRequirement, Requirements},
    Error, ErrorCode,
};
use std::collections::BTreeSet;

/// The current BDF adapter uses dense linear algebra. Symbolic Jacobian sparsity
/// does not qualify sparse factorization, and Jacobian products do not qualify
/// parameter gradients. Add support here only with executable evidence.
pub fn validate_requirements(requirements: &Requirements) -> Result<(), Error> {
    let unsupported = |name: &str| {
        let mut error = Error::new(
            ErrorCode::Unsupported,
            format!("model requires unsupported execution capability: {name}"),
        );
        error.expression = Some(format!("requirements.{name}"));
        error
    };
    // Both stiff and non-stiff equations may use the qualified BDF path. A false
    // requirement is no constraint; it does not require an explicit method.
    if requirements.sparse_jacobian {
        return Err(unsupported("sparse_jacobian"));
    }
    if requirements.gradients {
        return Err(unsupported("gradients"));
    }
    let mut seen = BTreeSet::new();
    for event in &requirements.events {
        if !seen.insert(*event) {
            let mut error = Error::new(
                ErrorCode::InvalidInput,
                "duplicate required event capability",
            );
            error.expression = Some("requirements.events".into());
            return Err(error);
        }
        match event {
            EventRequirement::Bolus
            | EventRequirement::Infusion
            | EventRequirement::Reset
            | EventRequirement::CovariateStep => {}
            EventRequirement::StateTriggered
            | EventRequirement::ParameterDependentTime
            | EventRequirement::SteadyState => {
                let mut error = unsupported(event.as_str());
                error.expression = Some("requirements.events".into());
                return Err(error);
            }
        }
    }
    Ok(())
}
