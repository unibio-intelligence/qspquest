# Use QSPQuest with your agent

QSPQuest supplies modeling tools through an MCP server. Connect your own agent
client, choose its model, and ask it to inspect models, simulate doses or fit
observations. QSPQuest runs the numerical calculations and returns structured
results with units. The container needs no AI account or provider configuration.

Purpose: connect an MCP client and complete a synthetic one-compartment
simulation. You need Docker Engine or Desktop running, an MCP client supporting
Streamable HTTP or stdio, and a QSPQuest MCP image for your machine's Linux
architecture: ARM64 for Apple Silicon, or AMD64 for most Intel/AMD machines.
Python 3 is optional for the connection check below.

## Get the image

With a prebuilt image, loading it and making the first connection takes a few
minutes. Registry images and public image archives are not yet distributed.
If you have a supplied `qspquest-mcp-arm64.tar.gz` or
`qspquest-mcp-amd64.tar.gz`, load the matching archive:

```sh
# Apple Silicon; use qspquest-mcp-amd64.tar.gz on Intel/AMD.
docker load --input qspquest-mcp-arm64.tar.gz
docker image inspect qspquest-mcp:local
```

Otherwise, build from the complete QSPQuest source root:

```sh
docker build --file bindings/mcp/Dockerfile --target public \
  --tag qspquest-mcp:local .
```

The first source build downloads and compiles locked dependencies. It can take
tens of minutes and is a preparation step; subsequent runs use the built image.
The image contains the server, isolated WASM engine, browser assets, license
notices and a small synthetic model. No model-library download is needed for
this first run.

## Connect over HTTP

Start the container in the background:

```sh
docker run --detach --name qspquest-mcp --init \
  --publish 127.0.0.1:8080:8080 --read-only \
  --cap-drop ALL --security-opt no-new-privileges \
  qspquest-mcp:local
```

In your agent client's MCP settings, add a server named `qspquest`, choose
**Streamable HTTP**, and set its URL to **`http://127.0.0.1:8080/mcp`**.
No bearer token is required for this local example. The listener is published
only on your computer's loopback address. Inputs and results travel between your
agent client and the container; your client controls what it sends to its AI
provider. This local setup does not accept connections from a hosted client
running on another machine.

A client that supports the `mcpServers` URL configuration can use:

```json
{
  "mcpServers": {
    "qspquest": {
      "url": "http://127.0.0.1:8080/mcp"
    }
  }
}
```

Some clients use a different configuration format. Use their Streamable HTTP
server setting with the same URL, or the stdio connection below. Restart or
reconnect the client after changing its MCP settings.

From the source root, an optional standard-library connection check discovers
the tools and runs the complete simulation:

```sh
python3 bindings/mcp/scripts/quickstart.py --url http://127.0.0.1:8080/mcp
```

Expect `checks_passed: true`, four sample times and the concentrations below.
This checks the actual server connection and numerical result without an AI
client.

## Ask your agent for the first simulation

Paste this prompt into the connected agent:

> Use the QSPQuest MCP tools. Search for `synthetic_one_compartment`, inspect
> its model card and units, and simulate a 6 mg bolus into `central` at time
> zero. End at 24 h and report post-dose `cp` at 0, 2, 8 and 24 h in mg/L.
> Use the tool's calculated values. Include its result status and explain
> the model's assumptions.

The agent should call `search_models`, `get_model`, then `simulate`.
The complete simulation arguments below explicitly request post-dose observations,
so the time-zero value is 2 mg/L. The native request schema retains its versioned
identifier:

```json
{
  "model": {
    "library": {
      "model_id": "synthetic_one_compartment"
    }
  },
  "request": {
    "schema": "qspquest.run/v0.1",
    "solver": "diffsol_bdf",
    "budgets": {
      "solver_callbacks": 1000000,
      "events": 100,
      "output_values": 10000
    },
    "regimen": {
      "end": {
        "value": 24,
        "unit": "h"
      },
      "samples": [],
      "observations": [
        {
          "time": {
            "value": 0,
            "unit": "h"
          },
          "side": "post"
        },
        {
          "time": {
            "value": 2,
            "unit": "h"
          },
          "side": "post"
        },
        {
          "time": {
            "value": 8,
            "unit": "h"
          },
          "side": "post"
        },
        {
          "time": {
            "value": 24,
            "unit": "h"
          },
          "side": "post"
        }
      ],
      "administrations": [
        {
          "target": "central",
          "time": {
            "value": 0,
            "unit": "h"
          },
          "amount": {
            "value": 6,
            "unit": "mg"
          },
          "delivery": {
            "kind": "bolus"
          },
          "bioavailability": 1.0
        }
      ],
      "rtol": 1e-10,
      "atol": 1e-12
    }
  },
  "outputs": [
    "cp"
  ]
}
```

| Time (h) | Concentration (mg/L) |
| ---: | ---: |
| 0 | 2.000000 |
| 2 | 1.637462 |
| 8 | 0.898658 |
| 24 | 0.181436 |

This synthetic model has volume 3 L and clearance 0.3 L/h, giving
`cp(t) = 2 * exp(-0.1 * t)`. It illustrates linear elimination from one
well-mixed compartment and supplies no clinical dosing recommendation.
Check `structuredContent.contract.status`, the returned units and event sides
before interpreting a result. A failed calculation is not a completed answer.
For a changed dose or parameter, ask the agent to call the tools again.

## Connect through stdio instead

For a local client that launches an MCP process, use this configuration.
Docker starts the container when the client connects; do not add `--detach`
or `--tty`, because MCP uses stdin and stdout for protocol messages.

```json
{
  "mcpServers": {
    "qspquest": {
      "command": "docker",
      "args": [
        "run", "--rm", "-i", "--network", "none", "--read-only",
        "--cap-drop", "ALL", "--security-opt", "no-new-privileges",
        "qspquest-mcp:local", "--profile", "local"
      ]
    }
  }
}
```

If the client cannot find Docker, replace `docker` with the absolute executable
path reported by `command -v docker` on macOS/Linux or `where docker` on
Windows. The server runs offline in this mode. You can check this exact
container command from the source root:

```sh
python3 bindings/mcp/scripts/quickstart.py --stdio-image qspquest-mcp:local
```

Expect the same four calculated concentrations and `checks_passed: true`.

## Add your models and stop the server

The image starts with the teaching model. To use an extracted model library or
your own directory of `.qsp` files, stop the HTTP container and mount that
directory read-only. Replace `/absolute/path/to/models` with a readable host
path; model-library roots contain `catalog.json` and `models/`.

```sh
docker rm --force qspquest-mcp
docker run --detach --name qspquest-mcp --init \
  --publish 127.0.0.1:8080:8080 --read-only \
  --cap-drop ALL --security-opt no-new-privileges \
  --mount type=bind,source=/absolute/path/to/models,target=/models,readonly \
  --env QSPQUEST_LIBRARY_DIR=/models qspquest-mcp:local
```

Search the newly selected library and use the IDs, units and stored scenario
requests it returns. The mounted library replaces the starter catalog.
Read [shared models](../site/shared-models.md) for selecting a library and
[MCP result handling](mcp-result-resources.md) for complete result retrieval.

Stop and remove the HTTP example when finished:

```sh
docker rm --force qspquest-mcp
```

If port 8080 is in use, publish `127.0.0.1:8081:8080` and change the client's
URL to `http://127.0.0.1:8081/mcp`. For startup errors, run
`docker logs qspquest-mcp`. Docker must be running and the image architecture
must match the host. Keep the server on loopback for this workflow; remote
clients need an operator-configured HTTPS deployment and access policy, as
described in [deployment profiles](mcp-deployment-profiles.md).
