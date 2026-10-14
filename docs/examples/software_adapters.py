"""Run an included synthetic source through every structural adapter export."""

import math
from qspquest.adapters import SOFTWARE, import_model, export_model, simulation_dataset

model = import_model(
    "{d/dt(central)=-cl/v*central;cp=central/v;}",
    "rxode2",
    time_unit="h",
    model_id="synthetic_adapter_example",
    parameters={"cl": 0.3, "v": 3},
    outputs={"cp": "cp"},
    dosing_targets=["central"],
)
rows = [{"ID": "synthetic", "TIME": 0, "EVID": 1, "CMT": 1, "AMT": 6}]
rows += [{"ID": "synthetic", "TIME": t, "EVID": 0} for t in (0, 1, 2, 4, 8)]
for software in SOFTWARE:
    exported = export_model(model, software)
    restored = exported.import_back()
    dataset = simulation_dataset(
        rows, restored, software="nonmem", end=8, rtol=1e-10, atol=1e-12
    )
    result = dataset.run()["results"]["synthetic"]
    error = max(
        abs(value - 2 * math.exp(-0.1 * time))
        for time, value in zip(result["times"], result["outputs"][0]["values"])
    )
    assert error < 2e-8
    print(software, "max absolute error:", error)
