# Declared execution requirements

The optional model `requirements` record states minimum execution capabilities.
Missing booleans default to false and missing events to an empty list. A false
value imposes no requirement. The list states capabilities the model needs; it
is not a whitelist forbidding other supported events in a run request. Actual
requests still undergo event, domain, unit and budget validation.

```text
requirements
  stiff = true
  sparse_jacobian = false
  gradients = false
  events = [bolus, infusion, reset, covariate_step]
```

The equivalent JSON record uses the same keys, boolean values and an array of
quoted event names. Unknown keys, unknown names and duplicate events fail.
Text declarations also reject duplicate keys and non-boolean flag values.

| Requirement | Current behavior |
|---|---|
| `stiff` | Supported through the BDF solver; no automatic solver selection |
| `sparse_jacobian` | Unsupported: the current adapter uses dense factorization |
| `gradients` | Unsupported: parameter sensitivities are not yet qualified |
| `bolus`, `infusion`, `reset`, `covariate_step` | Supported within the documented fixed-event contracts |
| `state_triggered`, `parameter_dependent_time`, `steady_state` | Unsupported |

The sparse requirement means an execution path that exploits a sparse Jacobian
in linear algebra. Existing sparse symbolic products do not satisfy it.
The gradient requirement refers to parameter gradients, not the directional
state Jacobian used by the implicit solver.

`qspquest_modeling::capabilities::validate_requirements` runs before equation compilation
and binding. Unsupported requirements return `unsupported` with the requirement
path; text diagnostics point to the declaration. This is a minimum-capability
check, not proof that arbitrary stiff models or nonsmooth expressions will solve.

Requirements are serialized as scientific model content and affect its hash.
The default record is canonicalized explicitly, so existing documents acquire
new model hashes under this compiler version. Historical execution
identities remain evidence of the compiler that produced them. No public stable
schema or cross-revision hash compatibility is claimed.

The formatter currently validates engine support before formatting, consistent
with the rest of the language subset. Unsupported requirements can be parsed
into the IR and structurally validated, but cannot pass compilation/formatting.

Declared `switches` now support the narrow simulation case where a switch time
is computed from parameters once per run. They lower to the qualified step
binding path. The broad `parameter_dependent_time` requirement remains
unsupported: it includes event-time behavior beyond these declarations, and
this feature does not qualify moving-time parameter sensitivities or generic
parameter-dependent dosing events.
