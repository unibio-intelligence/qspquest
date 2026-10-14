# Try QSPQuest in your browser


The browser quickstart runs the synthetic one-compartment model on your device.
Change dose, clearance or volume, select **Run simulation**, and explore the
concentration plot. Download the 97 post-dose observations as CSV. Browser
simulation uses a local WebAssembly worker; your own agent connects separately
through the [MCP Docker quickstart](tutorials/agents-docker.md).

## Run a prepared browser bundle

You need Python 3.10+ and a browser supporting WebAssembly and module workers.
Extract the prepared `qspquest-browser.zip`, then run this from its folder:

```sh
python3 -m http.server 8000 --bind 127.0.0.1
# Open http://127.0.0.1:8000
```

On Windows, use `py -m http.server 8000 --bind 127.0.0.1`. Select **Run simulation**.
The default 6 mg dose, 3 L volume and 0.3 L/h clearance give concentrations
of 2, 1.637462, 0.898658 and 0.181436 mg/L at 0, 2, 8 and 24 hours. Doubling the
dose doubles concentration. Increasing clearance makes the curve decline faster.
The model, complete request, client and legal notices are included in the bundle.
Stop the server with Ctrl+C. No Rust installation or remote execution service is
needed for the prepared bundle. A bundle must be prepared using the source build
below; these instructions do not assume that a downloadable release exists.

Use the documented default values for your first run. The controls accept positive
finite values within their displayed HTML bounds. Invalid values are rejected.
A changed input clears the previous result; run again to update it. **Cancel**
terminates the active worker, and a later run creates a fresh worker. A missing
asset reports an error; serve the complete extracted bundle over HTTP rather
than opening the HTML as a file. If port 8000 is occupied, choose another port
and open the matching address. The worker has a 30-second execution timeout.

## Build the browser quickstart from source

The browser client compiles and simulates a model in a Web Worker backed by
WebAssembly. This tutorial runs the included synthetic absorption model locally.
It requires the complete source release, Rust 1.98.1 with `rustup`, Python 3.10+
and a browser that supports module workers. It needs no model-library checkout.

From the source root, build the assets into the layout used by the worker:

```sh
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version 0.2.129 --locked
cargo build -p qspquest-wasm --release --locked --target wasm32-unknown-unknown
wasm-bindgen target/wasm32-unknown-unknown/release/qspquest_wasm.wasm --target web --out-dir bindings/wasm/pkg/engine
```

Serve the complete source root and open the supplied interactive page:

```sh
python3 -m http.server 8000 --bind 127.0.0.1
# Open http://127.0.0.1:8000/bindings/wasm/quickstart/
```

To prepare a standalone bundle for users or static hosting:

```sh
python3 scripts/build_browser_quickstart.py --output qspquest-browser.zip
```

The packager refuses to overwrite an existing archive. Use a new output filename
for another build. Hosting the extracted files on one HTTP(S) origin uses the
same relative asset layout; no backend is needed for simulation.

## Embed the worker in your own page

The worker accepts model sources up to 3,000,000 UTF-8 bytes and requests up to
1,000,000 bytes. The quickstart editor applies its own smaller input limit.
Solver, event and output budgets still apply. Completed runs retain one compiled
model: parameter and dosing changes reuse it, while a changed source replaces it.
Cancellation discards the worker and its compiled model.

Save the following complete example as `browser-example.html` in the source root:

```html
<!doctype html>
<html lang="en">
<meta charset="utf-8">
<title>QSPQuest synthetic absorption</title>
<h1>Synthetic absorption</h1>
<pre id="result">Running…</pre>
<script type="module">
import { createEngineClient } from './bindings/wasm/js/index.mjs';
const output = document.getElementById('result');
const engine = createEngineClient();
try {
  const response = await fetch('./examples/synthetic/absorption.qsp');
  if (!response.ok) throw new Error(`Model download failed: ${response.status}`);
  const source = await response.text();
  const quantity = (value, unit) => ({ value, unit });
  const request = {
    schema: 'qspquest.run/v0.1', solver: 'diffsol_bdf',
    budgets: { solver_callbacks: 1000000, events: 100, output_values: 10000 },
    regimen: {
      end: quantity(24, 'h'),
      samples: Array.from({ length: 25 }, (_, hour) => quantity(hour, 'h')),
      administrations: [{ target: 'depot', time: quantity(0, 'h'),
        amount: quantity(100, 'mg'), delivery: { kind: 'bolus' }, bioavailability: 1 }],
      rtol: 1e-8, atol: 1e-10
    }
  };
  const { result } = await engine.run({ kind: 'run', source, request, outputs: ['cp'] });
  output.textContent = JSON.stringify({ names: result.names,
    times: Array.from(result.times), sides: Array.from(result.sides),
    concentrations: Array.from(result.columns[0]) }, null, 2);
} catch (error) {
  output.textContent = `${error.code ?? 'error'}: ${error.message}`;
} finally {
  engine.dispose();
}
</script>
</html>
```

Serve the source root and open the page:

```sh
python3 -m http.server 8000 --bind 127.0.0.1
# Open http://127.0.0.1:8000/browser-example.html
```

The output names contain `cp`, in mg/L. Concentration starts at zero, rises as
100 mg enters the central compartment, then declines. At one hour it is about
10.76 mg/L, and at 24 hours about 0.7643 mg/L. The times and side codes align with
the concentration array; an administration can produce pre/post rows at the
same time. These are synthetic teaching values, not a clinical regimen.

Use HTTP rather than opening the HTML through `file://`; browsers otherwise
block module/asset loading. A missing `pkg/engine/qspquest_wasm.js` means the
binding-generator command did not finish or wrote into a different directory.
Keep the generator version above in sync with Cargo.lock. Serve JavaScript and
WASM from the same origin, and allow local Workers in your content security policy.
Stop the server with Ctrl-C when finished.

`source` may contain a `.qsp` model or a JSON model document. Requests use the
same versioned format as the command-line program. Result columns are typed
arrays aligned with `times`; `sides` identifies pre/post event rows. `run()`
returns a request ID and result, and accepts `timeoutMs` and `onStarted` options.
Call `dispose()` when the client is no longer needed. The worker accepts model
and request payloads of at most one million bytes each.

The public worker handles **simulation requests only**. The underlying WASM
binding also has sensitivity and fit entry points, but this worker does not
expose them. An embedding application would need to integrate those bindings
explicitly before offering browser-based fitting.

## Privacy boundary

For the supplied worker, model source and simulation requests are sent from
the page to its local Worker and processed by WASM on the user's device. The
worker does not upload them or the resulting trajectories to a QSPQuest
service. This allows a browser application to keep a user's model and data
local during simulation.

The browser still fetches the JavaScript and WASM assets. The embedding
application controls where those assets come from and whether it uploads
inputs or results, logs telemetry, or stores anything locally. Review those
application behaviors before promising users that their data never leaves
their device. Browser execution also does not replace application-level
access control or clinical validation.

## Use your own model

1. Choose **Your own model** in the model menu.
2. Open a QSPQuest `.qsp` model or a native JSON model document with **Open model**.
3. Open its complete native JSON simulation request with **Open run request**.
4. Select **Run simulation**, then choose a returned output in **Plot output**.

You can also paste and edit both inputs in **Edit model and run request**. To
try this with the included runnable example, open
`conformance/models/synthetic-one-compartment.qsp` and
`docs/examples/first-run.json` from the extracted bundle. The four observations
match the default analytic values above. Edit the request's administration amount
from 6 to 12 mg and run again; every concentration doubles. The previous run
appears as a dashed line when model identity, time range, output and units match.
**Restore example** clears the comparison and returns to the teaching controls.

The input files are read into browser memory rather than uploaded. Changes are
not saved back to the files. Copy edited source/request text if you need to keep
it. The request must use `qspquest.run/v0.1`; JSON models use the native
`qspquest.model/v0.1` schema. These versioned schema names remain supported
wire identifiers. A model file alone does not specify a dosing/sampling experiment.
Provide compatible states, parameter names and units in the request.

The browser page allows at most 20,000 output values, 1,000,000 solver callbacks,
10,000 events and one million bytes per input. Set the corresponding request
budgets within these limits. Errors show the engine diagnostic or the missing
input. Invalid JSON or model syntax can be corrected and rerun. Choose the native
CLI or MCP server for larger workloads or supported fitting. Direct OMEX, SBML
and arbitrary program imports are not accepted by this page.

## Include a model-library preset

A maintainer can add one exact model version and scenario from an extracted
QSPQuest model library to a browser bundle. For example, with the companion
library extracted beside the engine source:

```sh
python3 scripts/build_browser_quickstart.py \
  --library-root ../qspquest-models \
  --library-model qspquest_diffdose_2026_bite_benchmark \
  --library-version 0.1.0 \
  --library-scenario lower_exposure \
  --output qspquest-browser-bispecific.zip
```

The packager verifies archive and member checksums, checks that catalog, archive
and native model identities agree, and requires the selected scenario in both
the catalog and archive index. It extracts the native simulation projection and
request and includes the model's legal notices. The browser output budget is
reduced to 20,000, the event budget to 10,000 and callback budget to 1,000,000;
all model coordinates and scenario
values are preserved. A scenario exceeding those limits reports an error.

After extraction and serving as above, select **Bispecific exposure and ternary
binding**. Select a named parameter and edit its value, run the simulation, then
choose `drug_exposure`, `binary_complexes` or `ternary_complex`. These benchmark
outputs have declared dimensionless units (`1`), with time in days. A dashed
curve compares the previous compatible run. The default lower-exposure scenario
returns 564 observations and 13 outputs, including state readouts. CSV uses the
native output IDs and contains every output with its unit and pre/post event side;
the table displays at most 100 observations for the selected output.

The menu contains only the presets packaged in that bundle. It does not download
the entire library, contact GitHub during simulation, or imply browser support
for every catalog model. Check each selected model and scenario in the browser
before distributing its bundle. Respect model-specific licensing when hosting
or sharing those files.
