# Software adapters

`qspquest.adapters` imports and exports deterministic individual ODE models
for NONMEM, nlmixr2, rxode2, Pumas, Monolix Mlxtran, Phoenix PML and WinNonlin
ASCII. Every accepted import compiles with the same native QSPQuest engine.
The adapters do not call a vendor executable, fit populations, infer missing
units, evaluate source programs or open proprietary project containers.

This is a documented source subset, not general compatibility with every model
written for these tools. Unsupported statements raise `AdapterError` before
returning an executable model. A native compiler failure raises `QSPQuestError`.

| Source | Direct model import | Explicit exclusions |
|---|---|---|
| NONMEM | Named `$MODEL` compartments, ADVAN6/8/13 `$DES`, numeric `$THETA`, scalar `$PK`/`$DES`/`$ERROR` assignments, `A(n)`, `THETA(n)`, explicit `ETA_n` values and deterministic `Y`/`IPRED`; `F` requires an explicit `DEFOBS` compartment | Other ADVANs, procedural IF/loops/reassignments, `EPS`, modeled F/ALAG/R/D dosing attributes, `A_0`, priors and full estimation workflows |
| nlmixr2 | One `function()` with `ini({})` and `model({})`, numeric scalar initial estimates, named ODEs, initial conditions and algebraic readouts | Arbitrary R, bounded initialization vectors, `linCmt`, delay/history, procedural conditions and estimation conversion |
| rxode2 | ODE text or `rxode2({})`, named states, `state(0)` and scalar assignments; explicit parameter bindings | General R programs, event-generation calls, delay/history, modeled dosing attributes and implicit analytic models |
| Pumas | Explicit `@dynamics` ODEs, four supported linear PK dynamics, scalar `RealDomain` initial values/bindings, `@pre`, `@vars`, `@init`, deterministic `@derived`/`@observed` and scalar covariate bindings | Other built-in dynamics, reactions, dosecontrol, general Julia, matrix domains and population estimation |
| Monolix | `[LONGITUDINAL]`, named inputs, explicit ODEs, zero `t_0`, initial conditions, deterministic outputs and direct `depot(target=state)` | Full `.mlxtran` projects, `pkmodel`, absorption/lag/duration depot options, regressors, stochastic definitions and delays |
| Phoenix NLME | PML `name(){}`, explicit `deriv`, scalar `fixef`, `stparm`, scalar covariates and plain `dosepoint(state)` | Phoenix project archives, `fMicro`/other analytic macros, dosing options, procedural programs and population estimation |
| Phoenix WinNonlin | ASCII `COMMANDS`, `TEMPORARY`, `STARTING`, `DIFFERENTIAL`, one `FUNCTION`; numeric INITIAL/CONSTANTS and indexed P/CON/Z/DZ | Built-in model numbers, NCA configurations, PNAMES aliases, data transformations, multiple functions, command abbreviations and project archives |

The arithmetic subset includes addition, subtraction, multiplication, division,
powers, `exp`, `log`, `log10`, `sqrt`, trigonometric functions, `abs`, and binary
`min`/`max`. Unsupported comparisons, conditions and calls fail explicitly.
Limits are 1 MB source, 2000 symbols, 10000 characters/1000 nodes per expression,
and expression nesting of 32.
Pumas `Central1`, `Central1Periph1`, `Depots1Central1` and
`Depots1Central1Periph1` expand into their equivalent ODEs. This preserves the
structural model; QSPQuest runs its ODE solver rather than Pumas's closed form.

## Import and simulate

```python
from qspquest.adapters import import_model, simulation_dataset

model = import_model(
    "{ d/dt(central) = -cl/v*central; cp = central/v; }",
    "rxode2", model_id="synthetic_pk", time_unit="h",
    parameters={"cl": 0.3, "v": 3}, outputs={"cp": "cp"},
    dosing_targets=["central"],
)
dataset = simulation_dataset([
    {"ID": "synthetic", "TIME": 0, "EVID": 1, "CMT": 1, "AMT": 6},
    {"ID": "synthetic", "TIME": 0, "EVID": 0},
    {"ID": "synthetic", "TIME": 8, "EVID": 0},
], model, end=8)
answer = dataset.run()
print(answer["results"]["synthetic"]["outputs"][0]["values"])
```

**Units are numerical coordinates.** State, parameter, dose and output units are
`1`, with the explicitly supplied source time unit `s`, `min`, `h` or `d`
(`day` is accepted as an alias for `d`). The example retains the original numbers;
it does not qualify their physical units as mg, L or mg/L. The imported model's
fixed clock basis preserves the numeric derivative and `t` across time units.
Use a native unit-bearing model or SBML/OMEX when physical-unit interchange is
required. Inspect `warnings`, `symbol_map`, `output_map`, `compartment_map` and
`source_sha256` before using an import. Source identifiers map to stable native
identifiers without case/Unicode collisions.

Statistical declarations are rejected by default. Explicit
`structural_only=True` retains supported-block declarations as unexecuted source
metadata. It does not sample random effects or translate a likelihood. Supply
every individual effect and scalar covariate used in the equations explicitly,
for example `parameters={"eta.cl": 0.1}` for nlmixr2,
`{"ETA_1": 0.1}` for NONMEM or `{"eta_1": 0.1}` for Pumas `eta[1]`.
There is no automatic zero-effect assumption. Select deterministic readouts
explicitly with `outputs` when the source contains only statistical observations.

## Event tables

`simulation_dataset` accepts row dictionaries or a pandas data frame. NONMEM,
nlmixr2 and rxode2 default to uppercase NONMEM headers; Pumas defaults to lowercase
headers. Monolix, Phoenix and WinNonlin require explicit column mappings because
their exports have no single universal event-table convention:

```python
columns = {"ID": "subject", "TIME": "time", "EVID": "event",
           "AMT": "dose", "CMT": "compartment", "DV": "concentration"}
# dataset = simulation_dataset(rows, model, software="phoenix",
#                              columns=columns, end=24)
```

Each row must explicitly identify ID, TIME and EVID. EVID 1 is a positive dose;
RATE 0 is a bolus and positive RATE is a finite infusion. ADDL/II expands fixed
repeat doses. EVID 0 with MDV 0 records a sampling time; MDV 1 retains an excluded
source row. Every subject needs an observation. Observations must be nondecreasing
within subjects; duplicates are retained. Same-time sampling uses the explicit
global `observation_side="pre"` or `"post"`, independent of row order.

Only ID, TIME, EVID, DV, MDV, AMT, CMT, RATE, ADDL, II and SS may be mapped. Censoring,
reset events, nonzero SS, negative RATE codes, unmapped columns and time-varying
covariates are rejected. Templates never silently extend the requested horizon.
DV and observation CMT are preserved as source metadata; simulation returns every
selected readout at each sample and does not infer an observation likelihood or
endpoint assignment. Native population fitting uses the separate explicit
`population_fit_dataset` endpoint/error contract.

`export_dataset(dataset, software, columns=...)` returns source-order records
with the chosen destination headers. It preserves source columns and dose codes;
it is not a complete vendor dataset or project generator. Dose and observation
source rows remain available in `dataset.rows`, with per-subject result indexes.

## Structural export

```python
from qspquest.adapters import export_model
exported = export_model(model, "pumas")
print(exported.source)
restored = exported.import_back()
```

Exports accept an `AdaptedModel`, retain numerical equations/values and provide
explicit parameters, outputs and dosing-target bindings. Preserve the complete
`ExportedModel` alongside its text: Mlxtran/rxode2 require parameter bindings,
and other formats need the explicit readout/dosing selections. Exports rename
source symbols and exclude statistical/estimation metadata; they do not promise
textual round trips or produce full fitting projects. NONMEM and WinNonlin export
support one readout. Nonzero initial states are preserved in rxode2/nlmixr2/Pumas/
Mlxtran/WinNonlin; direct NONMEM and Phoenix export rejects them. General native
physical-unit models should use `qspquest.interchange` instead.
Source bounds are retained for Pumas, NONMEM and Phoenix export. Export to the
other source subsets rejects bounded parameters because those subsets cannot
retain the bounds faithfully.

## Optional existing-converter bridge

Existing external converters can handle more source syntax before the shared
QSPQuest importer checks its own supported subset. No converter code is bundled
in QSPQuest. Install and use those packages in a separate R environment; their
licenses and dependencies remain independent.

```r
source("bindings/r/R/software-adapters.R")
# A supported nlmixr2/rxode2 UI can be passed directly.
# For a trusted NONMEM control stream:
ui <- nonmem2rx::nonmem2rx("model.ctl", validate=FALSE, nonmemData=FALSE,
                         updateFinal=FALSE, save=FALSE, load=FALSE,
                         usePhi=FALSE, useExt=FALSE, useCov=FALSE,
                         useXml=FALSE, useLst=FALSE, matexp=FALSE)
qspquest_export_rxode2_ui(ui, "rxode2-model.json")
# A Monolix project can instead be read with monolix2rx::monolix2rx(...).
# That converter can read project-linked files and perform reference simulations;
# run it only in an environment appropriate for the trusted project.
```

```python
from pathlib import Path
from qspquest.adapters import import_rxode2_snapshot
model = import_rxode2_snapshot(Path("rxode2-model.json").read_text(),
                              time_unit="h", structural_only=True,
                              parameters={"eta.cl": 0}, outputs={"ipred": "ipred"})
```

The snapshot records the source function, initial fixed-effect values, compartment
order and installed package versions. Import validates state order and rejects
unsupported residual/dosing/history constructs. It does not make every model
accepted by nonmem2rx/monolix2rx acceptable to QSPQuest. The helper has been
exercised on an actual rxode2 UI; NONMEM and Monolix converter executions require
separate qualification in an environment with those packages.

The relevant primary documentation is [NONMEM Users Guide VIII](https://nonmem.iconplc.com/nonmem744/guides/VIII.pdf),
[nlmixr2](https://nlmixr2.org/), [Pumas model syntax](https://docs.pumas.ai/stable/basics/models/),
[Mlxtran ODE models](https://monolixsuite.slp-software.com/monolix/2024R1/time-varying-clearance),
[Phoenix PML](https://onlinehelp.certara.com/phoenix/8.2/topics/pmlexamples.htm),
[WinNonlin ASCII](https://onlinehelp.certara.com/phoenix/8.7/Phoenix_UserDocs/WNL_Core_Output_File/Commands-Arrays-Functions.htm),
[nonmem2rx](https://nlmixr2.github.io/nonmem2rx/reference/nonmem2rx.html) and
[monolix2rx](https://nlmixr2.github.io/monolix2rx/reference/monolix2rx.html).

## Checks and qualification boundary

Run `python -m pytest bindings/python/tests/test_software_adapters.py` for
independent analytic PK, explicit individual effects, event and rejection checks.
`bindings/r/tests/software-adapters.R` additionally evaluates generated rxode2 and
nlmixr2 syntax in an actual rxode2 runtime:

```sh
python bindings/python/tests/test_software_adapters.py /tmp/qspquest-adapter-checks
Rscript bindings/r/tests/software-adapters.R /tmp/qspquest-adapter-checks
```

The synthetic parameters and source
models were written for these checks; they are not extracted vendor/paper models.

These checks establish the tested deterministic subset. They do not establish
full vendor compatibility, equivalent population estimation, NCA workflows or
agreement with licensed NONMEM/Pumas/Phoenix/Monolix executables.
