"""General table bundles round-trip through the documented reader, including count units."""
import importlib.util
from pathlib import Path

import pytest
import qspquest as qq
from qspquest.result_bundle import write_table

ROOT = Path(__file__).resolve().parents[3]
SPEC = importlib.util.spec_from_file_location('table_bundle', ROOT / 'docs/examples/table_bundle.py')
table_bundle = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(table_bundle)

SOURCE = """model count_unit_bundle
time_unit = d
parameters
  cl = 0.1 [L/d] log
  v = 3 [L] log
  kill = 0.01 [L/(mg.d)] log
  base = 200 [[cell]/mm3] log
  kout = 0.05 [1/d] log
states
  central = 0 [mg]
  cells = (base) [[cell]/mm3]
dynamics
  d/dt(central) = input_central - cl / v * central
  d/dt(cells) = base * kout - kill * central / v * cells - kout * cells
outputs
  cp = central / v [mg/L]
  cells_out = cells [[cell]/mm3]
dosing
  central [mg] = 1
end
"""


def q(value, unit):
    return {'value': value, 'unit': unit}


def request():
    return {'schema': 'qspquest.run/v0.1', 'solver': 'diffsol_bdf',
            'budgets': {'solver_callbacks': 1_000_000, 'events': 100, 'output_values': 10_000},
            'regimen': {'end': q(20, 'd'), 'samples': [q(t, 'd') for t in (0, 5, 10, 20)],
                        'administrations': [{'target': 'central', 'time': q(10, 'd'), 'amount': q(100, 'mg'),
                                             'delivery': {'kind': 'bolus'}, 'bioavailability': 1.0}],
                        'rtol': 1e-8, 'atol': 1e-10}}


def test_count_unit_result_round_trips(tmp_path):
    run = request()
    result = qq.CompiledModel(SOURCE).run(run)
    written = write_table(tmp_path / 'bundle', result, SOURCE, run, name='cells', model_id='count_unit_bundle')
    value = table_bundle.read_table(written['table'])
    columns = {c['name']: c['unit'] for c in value['manifest']['table']['columns']}
    assert columns['cells_out'] == '[cell]/mm3' and columns['cp'] == 'mg/L'
    assert len(value['rows']) == len(result['times'])
    assert value['manifest']['events']['event_pairs'] == 1  # the dose at day 10 adds pre and post rows
    assert value['manifest']['qualification']['qualified'] is False
    assert value['rows'][-1]['cells_out'] == result['outputs'][1]['values'][-1]


def test_selected_outputs_and_refusals(tmp_path):
    run = request()
    result = qq.CompiledModel(SOURCE).run(run)
    written = write_table(tmp_path / 'one', result, SOURCE, run, outputs=['cp'])
    assert table_bundle.read_table(written['table'])['manifest']['inputs']['selected_outputs'] == ['cp']
    with pytest.raises(ValueError, match='unique names'):
        write_table(tmp_path / 'two', result, SOURCE, run, outputs=['missing'])
    with pytest.raises(FileExistsError):
        write_table(tmp_path / 'one', result, SOURCE, run)
    with pytest.raises(ValueError, match='plain file stem'):
        write_table(tmp_path / 'three', result, SOURCE, run, name='../escape')
