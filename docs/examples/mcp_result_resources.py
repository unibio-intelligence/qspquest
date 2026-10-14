"""Verify synthetic MCP trajectories or compact model cards and their resources."""
import copy
import hashlib
import io
import json
import math
import os
import re
from pathlib import Path
import queue
import threading
import subprocess
import sys
import tempfile
import zipfile

from table_bundle import read_table


def response_guard_example(rpc, tool, full_resource, source, request, origin):
    """Copy computed concentrations with explicit conditions and event sides."""
    runs, fields, complete = [], [], {}
    analytical_values = 0
    for label, amount, dose_times in [('single', 6, [0]), ('A', 6, [0, 2]), ('B', 12, [0, 2])]:
        scenario = copy.deepcopy(request)
        scenario['regimen']['end'] = {'value': 4, 'unit': 'h'}
        scenario['regimen']['observations'] = [
            {'time': {'value': t, 'unit': 'h'}, 'side': side}
            for t in [0, 2, 4] for side in ['pre', 'post']]
        scenario['regimen']['administrations'] = [
            {'target': 'central', 'amount': {'value': amount, 'unit': 'mg'},
             'time': {'value': t, 'unit': 'h'}, 'delivery': {'kind': 'bolus'}, 'bioavailability': 1}
            for t in dose_times]
        result = tool('simulate', {'model': {'source': source.replace(' cp =', ' library_state_0 =')},
                                 'request': scenario, 'outputs': ['library_state_0']})
        full = full_resource(result['full_result_resource'])
        complete[label] = full
        column = next(c for c in full['outputs'] if c['name'] == 'library_state_0')
        for t, side, value in zip(full['times'], full['sides'], column['values']):
            expected = sum(amount / 3 * math.exp(-0.1 * (t - d))
                           for d in dose_times if d < t or d == t and side == 'post')
            assert abs(value - expected) <= 1e-8 + 1e-7 * abs(expected)
            analytical_values += 1
        # Preserve the actual request identity rather than relying on a label.
        runs.append({'label': label, 'source': {'kind': 'retained', 'uri': result['full_result_resource']},
                     'expected_effective_request_hash': full['effective_request_hash']})
        for side in ['pre', 'post', 'both']:
            fields.append({'key': label + '_' + side + '_concentration_mg_L', 'run': label,
                           'column': 'library_state_0', 'unit': 'mg/L', 'time_unit': 'h',
                           'times': [0, 2, 4], 'event_side': side})
    arguments = {'runs': runs, 'fields': fields, 'identifiers': {'decision_id': 'dose-side-comparison'}}
    formatted = tool('format_response', arguments)
    answer, guard = formatted['formatted_response'], formatted['response_guard']
    assert guard['check'] == 'constructed' and guard['current_engine_and_request'] == 'not_assessed'
    assert answer['decision_id'] == 'dose-side-comparison'
    for receipt in guard['fields']:
        full = complete[receipt['run']]
        column = full['outputs'][0]['values']
        values = [column[point['source_row']] for point in receipt['observations']]
        assert answer[receipt['key']] == values and all(type(v) in (float, int) for v in values)
        assert all(point['side'] == full['sides'][point['source_row']] and
                   point['time'] == full['times'][point['source_row']] for point in receipt['observations'])
        assert len(values) == (len(full['times']) if receipt['requested_event_side'] == 'both' else 3)
    assert all(abs(b - 2 * a) <= 1e-8 + 1e-7 * abs(b) for a, b in
               zip(answer['A_post_concentration_mg_L'], answer['B_post_concentration_mg_L']))
    checked = tool('format_response', dict(arguments, candidate=answer))
    assert checked['response_guard']['check'] == 'candidate_matches'
    # A retrieved JSON body is pinned caller data; its digest does not authenticate a producer.
    supplied = copy.deepcopy(arguments)
    for run in supplied['runs']:
        raw = json.dumps(complete[run['label']], allow_nan=False)
        run['source'] = {'kind': 'supplied', 'json': raw, 'sha256': hashlib.sha256(raw.encode()).hexdigest()}
    assert tool('format_response', supplied)['formatted_response'] == answer
    assert all(s['origin'] == origin for s in guard['sources'])
    if origin == 'retained_run':
        cached = copy.deepcopy(arguments)
        for run in cached['runs']:
            run['source']['uri'] = 'qspquest://runs/' + complete[run['label']]['run_id']
        assert tool('format_response', cached)['formatted_response'] == answer

    refusals = 0
    def refuse(bad, code):
        nonlocal refusals
        reply = rpc('tools/call', {'name': 'format_response', 'arguments': bad})
        assert reply.get('isError') and reply['structuredContent']['error']['code'] == code, reply
        assert 'formatted_response' not in reply['structuredContent']
        refusals += 1
    bad = copy.deepcopy(arguments); bad['runs'].pop(); refuse(bad, 'response_run_completeness')
    bad = copy.deepcopy(arguments); bad['fields'][0]['column'] = 'uncomputed'; refuse(bad, 'response_output_missing')
    bad = copy.deepcopy(arguments); bad['fields'][0]['times'] = [1]; refuse(bad, 'response_time_not_computed')
    bad = copy.deepcopy(arguments); bad['fields'][0]['unit'] = 'kg/L'; refuse(bad, 'response_declared_units')
    bad = copy.deepcopy(arguments); bad['fields'][0]['expected_analyte'] = 'undeclared'; refuse(bad, 'response_output_semantics')
    bad = copy.deepcopy(arguments); bad['runs'][0]['source'] = arguments['runs'][1]['source']; refuse(bad, 'response_source_identity')
    for mutation in ['missing_key', 'extra_key', 'identifier', 'event_object', 'boolean', 'zero']:
        candidate = copy.deepcopy(answer)
        key = 'A_post_concentration_mg_L'
        if mutation == 'missing_key': del candidate['B_post_concentration_mg_L']
        if mutation == 'extra_key': candidate['unexpected'] = [0]
        if mutation == 'identifier': candidate['decision_id'] = 'different-decision'
        if mutation == 'event_object': candidate[key][0] = {'pre_dose': 0, 'post_dose': 2}
        if mutation == 'boolean': candidate[key][0] = True
        if mutation == 'zero': candidate[key][0] = 0
        refuse(dict(arguments, candidate=candidate),
               'response_candidate_keys' if mutation in ['missing_key', 'extra_key'] else 'response_candidate_value')
    bad = copy.deepcopy(supplied); bad['runs'][0]['source']['sha256'] = '0' * 64; refuse(bad, 'response_source_digest')
    bad = copy.deepcopy(supplied); full = copy.deepcopy(complete['single'])
    # Keep only a post-dose observation at the first bolus. A requested pre side must fail.
    for key in ['times', 'sides']: full[key].pop(0)
    for column in full['outputs']: column['values'].pop(0)
    raw = json.dumps(full); bad['runs'][0]['source'] = {'kind':'supplied', 'json':raw, 'sha256':hashlib.sha256(raw.encode()).hexdigest()}
    refuse(bad, 'response_event_side_missing')
    for mutation in ['preview', 'failed', 'nonconverged', 'event_object']:
        bad = copy.deepcopy(supplied); full = copy.deepcopy(complete['single'])
        if mutation == 'preview': full['trajectory_kind'] = 'bounded_preview'
        if mutation == 'failed': full['error'] = {'code': 'failed'}
        if mutation == 'nonconverged': full['converged'] = False
        if mutation == 'event_object': full['outputs'][0]['values'][0] = {'pre': 0, 'post': 2}
        raw = json.dumps(full); bad['runs'][0]['source'] = {'kind': 'supplied', 'json': raw, 'sha256': hashlib.sha256(raw.encode()).hexdigest()}
        refuse(bad, 'response_source_shape' if mutation == 'event_object' else 'response_source_incomplete')
    bad = copy.deepcopy(arguments); bad['runs'][0]['source']['uri'] = '/tmp/result.json'; refuse(bad, 'response_source_uri')
    print(f'Verified response guard: {analytical_values} analytical observations, three computed conditions, nine explicit column/key/side mappings, {origin}, pinned supplied JSON, and {refusals} refusals.')


def main():
    artifacts = Path(sys.argv[3]).resolve() if len(sys.argv) == 4 and sys.argv[2] == "--artifacts" else None
    required_outputs = len(sys.argv) == 3 and sys.argv[2] == "--required-outputs"
    dose_events = len(sys.argv) == 3 and sys.argv[2] == "--dose-events"
    linear_pk_doses = len(sys.argv) == 3 and sys.argv[2] == "--linear-pk-doses"
    time_validation = len(sys.argv) == 3 and sys.argv[2] == "--time-validation"
    overrides = len(sys.argv) == 3 and sys.argv[2] == "--overrides"
    scan = len(sys.argv) == 3 and sys.argv[2] == "--scan"
    patient = len(sys.argv) == 3 and sys.argv[2] == "--patient"
    precision = len(sys.argv) == 3 and sys.argv[2] == "--precision-profiles"
    scheduled_boundaries = len(sys.argv) == 3 and sys.argv[2] == "--scheduled-boundaries"
    scientific = len(sys.argv) == 3 and sys.argv[2] == "--scientific-assessments"
    response_guard = len(sys.argv) == 3 and sys.argv[2] == "--response-guard"
    initialization = len(sys.argv) == 3 and sys.argv[2] == "--initialization"
    comparison = len(sys.argv) == 3 and sys.argv[2] == "--comparison"
    source_families = len(sys.argv) == 3 and sys.argv[2] == "--source-families"
    model_pages = len(sys.argv) == 3 and sys.argv[2] == "--model-pages"
    inspection_selection = len(sys.argv) == 3 and sys.argv[2] == "--inspection-selection"
    input_bindings = len(sys.argv) == 3 and sys.argv[2] == "--input-bindings"
    model_cards = len(sys.argv) == 3 and sys.argv[2] == "--model-cards"
    steady_state = len(sys.argv) == 3 and sys.argv[2] == "--steady-state"
    morris = len(sys.argv) == 3 and sys.argv[2] == "--morris"
    documentation = len(sys.argv) == 3 and sys.argv[2] == "--documentation"
    csv_output = Path(sys.argv[3]).resolve() if len(sys.argv) == 4 and sys.argv[2] == "--csv-output" else None
    if not Path(sys.argv[1] if len(sys.argv) > 1 else "").is_file() or not (len(sys.argv) == 2 or source_families or model_cards or model_pages or inspection_selection or input_bindings or documentation or morris or steady_state or precision or initialization or scheduled_boundaries or scientific or response_guard or comparison or patient or scan or overrides or time_validation or required_outputs or dose_events or linear_pk_doses or csv_output or artifacts):
        raise SystemExit("Usage: python3 docs/examples/mcp_result_resources.py PATH_TO_QSPQUEST_MCP [--inspection-selection | --response-guard | --scientific-assessments | --scheduled-boundaries | --initialization | --precision-profiles | --source-families | --model-cards | --model-pages | --input-bindings | --documentation | --morris | --steady-state | --comparison | --patient | --scan | --overrides | --time-validation | --required-outputs | --dose-events | --linear-pk-doses | --csv-output OUTPUT_DIRECTORY | --artifacts PRIVATE_DIRECTORY]")
    source = """model synthetic_resource_example
 time_unit = h
parameters
 cl = 0.3 [L/h] fixed
 v = 3 [L] fixed
states
 central = 0 [mg]
dynamics
 d/dt(central) = input_central - cl / v * central
dosing
 central [mg] = 1
outputs
 cp = central / v [mg/L]
end"""
    if initialization:
        source = """model synthetic_protocol_initialization
time_unit = h
parameters
 synthesis = 2 [mg/h] fixed
 clearance = 1 [1/h] estimated
 reference = 2 [mg] estimated
individual
 condition_reference = synthesis / clearance [mg]
states
 target = 0 [mg]
 marker = 0 [mg]
dynamics
 d/dt(target) = synthesis - clearance * target
 d/dt(marker) = input_marker
dosing
 marker [mg] = 1
outputs
 fixed_pct = 100 * target / reference [1]
 condition_pct = 100 * target / condition_reference [1]
end"""
    if precision:
        source = source.replace(" cl = 0.3 [L/h] fixed", " cl = 0.3 [L/h] estimated")
    if required_outputs:
        source = source.replace(" cp = central / v [mg/L]", " cp = central / v [mg/L]\n synthetic_zero = (central - central) / v [mg/L]")
    if inspection_selection:
        source = source.replace(" cp = central / v [mg/L]", '\n'.join(
            f" cp_{i:03d} = central / v [mg/L]" for i in range(100)))
    if overrides:
        source = source.replace(" v = 3 [L] fixed", " v = 3 [L] fixed\n source_scale = 1 [1] fixed")
    request = {"schema": "qspquest.run/v0.1", "solver": "diffsol_bdf",
               "budgets": {"solver_callbacks": 1000000, "events": 100, "output_values": 100000},
               "regimen": {"start": {"value": 0, "unit": "h"}, "samples": [],
                           "end": {"value": 30, "unit": "h"},
                           "rtol": 1e-10, "atol": 1e-12,
                           "administrations": [{"target": "central", "amount": {"value": 6, "unit": "mg"},
                                                "time": {"value": 0, "unit": "h"},
                                                "delivery": {"kind": "bolus"}, "bioavailability": 1}],
                           "observations": [{"time": {"value": i / 10, "unit": "h"}, "side": "post"}
                                            for i in range(301)]}}
    scratch = tempfile.TemporaryDirectory(prefix="qspquest-example-")
    if response_guard:
        artifacts = Path(scratch.name) / 'artifacts'
    Path(scratch.name, "example.qsp").write_text(source)
    Path(scratch.name, "example.request.json").write_text(json.dumps(request))
    # Use only the supplied synthetic library, independent of installed settings.
    environment = {k: v for k, v in os.environ.items() if not k.startswith("QSPQUEST_")}
    if precision or initialization or scheduled_boundaries or scientific:
        csv_output = Path(scratch.name) / "csv"
    command = [str(Path(sys.argv[1]).resolve()), "--library-dir", scratch.name]
    if csv_output:
        command.extend(["--output-dir", str(csv_output)])
    if artifacts:
        command.extend(["--artifact-dir", str(artifacts)])
    def start():
        process = subprocess.Popen(command, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                   stderr=subprocess.PIPE, text=True, bufsize=1, env=environment)
        responses = queue.Queue()
        def receive():
            try:
                while True:
                    line = process.stdout.readline(16 * 1024 * 1024 + 1)
                    if not line:
                        code = process.wait(timeout=5)
                        diagnostic = process.stderr.read(4096).strip()
                        raise EOFError(f"MCP server stopped (exit {code}): {diagnostic or 'no startup diagnostic'}")
                    if len(line.encode()) > 16 * 1024 * 1024 or not line.endswith("\n"):
                        raise RuntimeError("MCP response exceeded its line budget")
                    responses.put(json.loads(line))
            except Exception as error:
                responses.put(error)
        threading.Thread(target=receive, daemon=True).start()
        return process, responses
    process, responses = start()
    request_id = 0

    def rpc(method, params, notification=False):
        nonlocal request_id
        request_id += 1
        message = {"jsonrpc": "2.0", "method": method, "params": params}
        if not notification:
            message["id"] = request_id
        process.stdin.write(json.dumps(message) + "\n")
        process.stdin.flush()
        if notification:
            return
        try:
            response = responses.get(timeout=60)
        except queue.Empty as error:
            raise RuntimeError("MCP response timed out; inspect your server configuration.") from error
        if isinstance(response, Exception):
            raise RuntimeError("MCP response failed: " + str(response)) from response
        if "error" in response:
            raise RuntimeError(response["error"])
        return response["result"]

    def read(uri):
        return "".join(part["text"] for part in rpc("resources/read", {"uri": uri})["contents"])

    def full_text(uri):
        text = read(uri)
        try:
            manifest = json.loads(text)
        except ValueError:
            return text
        if isinstance(manifest, dict) and manifest.get("schema") == "qspquest.result-artifact/v0.1":
            text = "".join(read(manifest["chunk_uri_prefix"] + str(index)) for index in range(manifest["chunk_count"]))
            raw = text.encode("utf-8")
            if len(raw) != manifest["bytes"] or hashlib.sha256(raw).hexdigest() != manifest["sha256"]:
                raise RuntimeError("Stored result size or chunk digest does not match.")
        if isinstance(manifest, dict) and manifest.get("kind") == "utf8_chunk_manifest":
            text = "".join(read(chunk) for chunk in manifest["chunks"])
            if hashlib.sha256(text.encode()).hexdigest() != manifest["sha256"]:
                raise RuntimeError("Resource chunk digest does not match.")
        return text

    def full_resource(uri):
        return json.loads(full_text(uri))

    def tool(name, arguments):
        response = rpc("tools/call", {"name": name, "arguments": arguments})
        result = response.get("structuredContent")
        if not isinstance(result, dict):
            raise RuntimeError("The server returned no structured result; check its version.")
        contract = result["contract"]
        assert contract["schema"] == "qspquest.mcp-result/v1"
        assert contract["library_version"] == result["library_version"]
        if contract["status"] != "success":
            raise RuntimeError(contract.get("failure", contract))
        assert not response.get("isError") and "error" not in result
        return result

    try:
        rpc("initialize", {"protocolVersion": "2025-11-25", "capabilities": {},
                           "clientInfo": {"name": "Trajectory example", "version": "1"}})
        rpc("notifications/initialized", {}, notification=True)
        capabilities = json.loads(read("qspquest://capabilities"))
        tools = rpc("tools/list", {})["tools"]
        assert capabilities["schema"] == "qspquest.mcp-capabilities/v1"
        assert capabilities["deployment"]["mode"] == "local_stdio"
        assert capabilities["deployment"]["csv_export_enabled"] == bool(csv_output)
        assert set(capabilities["tools"]) == {item["name"] for item in tools}
        assert all(item.get("outputSchema") for item in tools)
        if inspection_selection:
            identifier = "synthetic_resource_example"
            selected = tool("get_model", {"model_id": identifier, "selection": {
                "outputs": ["cp_097"], "scenarios": ["default"]}})["model_selection"]
            digest = selected["context"]["inspection_hash"]
            card = tool("get_model", {"model_id": identifier})
            assert selected["context"] == card["inspection"]["context"]
            offset = 0
            while True:
                page = tool("get_model", {"model_id": identifier, "page": {
                    "section": "outputs", "offset": offset, "expected_inspection_hash": digest}})["model_page"]
                matching = [row for row in page["entries"] if row["canonical_name"] == "cp_097"]
                if matching:
                    assert selected["outputs"] == matching
                    break
                assert page["next_offset"] is not None
                offset = page["next_offset"]
            scenario = tool("get_scenario_request", {"model_id": identifier,
                "scenario_id": "default", "expected_inspection_hash": digest})
            assert selected["scenarios"][0]["request"] == scenario["request"] == request
            mapping = selected["output_mapping"][0]
            assert mapping["requested_output"] == mapping["canonical_output"] == mapping["returned_column"] == "cp_097"
            assert mapping["returned_unit"] == "mg/L"
            uris = [selected["index_resource"], page["page_resource"], selected["outputs"][0]["record_resource"]]
            individual = [full_resource(uri) for uri in uris]
            batched = tool("get_model", {"model_id": identifier, "expected_inspection_hash": digest,
                "selection": {"resources": uris}})["model_selection"]
            assert [resource["value"]["data"] for resource in batched["resources"]] == individual
            # Planning and both execution routes use the exact same source and stored request.
            results = []
            for inputs in [selected["scenarios"][0]["request"], scenario["request"]]:
                arguments = {"model": {"library": {"model_id": identifier,
                    "expected_inspection_hash": digest}}, "request": inputs, "outputs": ["cp_097"]}
                plan = tool("plan_run", {"operation": "simulate", "arguments": arguments})
                arguments["expected_plan_hash"] = plan["plan_hash"]
                result = tool("simulate", arguments)
                results.append(full_resource(result["full_result_resource"]))
            assert results[0]["effective_request_hash"] == results[1]["effective_request_hash"]
            assert results[0]["outputs"] == results[1]["outputs"] and results[0]["times"] == results[1]["times"]
            assert all(abs(value - 2 * math.exp(-0.1 * t)) < 2e-7
                       for t, value in zip(results[0]["times"], results[0]["outputs"][0]["values"]))
            bad_batches = [
                ({"outputs": ["cp_097", "missing"]}, "inspection_output"),
                ({"outputs": ["cp_097", "cp_097"]}, "inspection_selection_names"),
                ({"scenarios": ["default", "missing"]}, "inspection_scenario"),
                ({"resources": [uris[0], "file:///tmp/private"]}, "inspection_selection_resource"),
                ({}, "inspection_selection_limit"),
                ({"outputs": [f"cp_{i:03d}" for i in range(33)]}, "inspection_selection_limit"),
                ({"resources": [uris[0].replace(digest[7:], '0' * 64)]}, "inspection_changed"),
            ]
            for selection, code in bad_batches:
                reply = rpc("tools/call", {"name": "get_model", "arguments": {
                    "model_id": identifier, "selection": selection}})
                assert reply.get("isError") and reply["structuredContent"]["error"]["code"] == code
                assert "model_selection" not in reply["structuredContent"]
            print("Verified exact output/scenario selection, three batched resources, identical requests and 301 analytical concentrations, and seven whole-batch refusals. Synthetic illustration; no model qualification or timing claim.")
            return
        if response_guard:
            response_guard_example(rpc, tool, full_resource, source, request, 'retained_artifact')
            process.terminate()
            process.wait(timeout=5)
            command = command[:-2]
            process, responses = start()
            rpc('initialize', {'protocolVersion':'2025-11-25', 'capabilities':{},
                              'clientInfo':{'name':'Response guard cache example','version':'1'}})
            rpc('notifications/initialized', {}, notification=True)
            rpc('tools/list', {})
            response_guard_example(rpc, tool, full_resource, source, request, 'retained_run')
            return
        if scientific:
            example = """model synthetic_flow_assessment
time_unit = h
parameters
 transfer = 1 [1/h] estimated
states
 central = 1 [mg]
 peripheral = 0 [mg]
dynamics
 d/dt(central) = -2 * transfer * central
 d/dt(peripheral) = transfer * central
outputs
 central_amount = central [mg]
 peripheral_amount = peripheral [mg]
end"""
            model = tool('validate_model', {'source':example})['model']
            native = json.dumps(model,sort_keys=True,allow_nan=False).encode()
            scenario = {'schema':'qspquest.run/v0.1','solver':'diffsol_bdf',
                'budgets':{'solver_callbacks':1000000,'events':100,'output_values':100000},
                'regimen':{'start':{'value':0,'unit':'h'},'end':{'value':2,'unit':'h'},
                    'rtol':1e-10,'atol':1e-12,'samples':[],'administrations':[],
                    'observations':[{'time':{'value':t,'unit':'h'},'side':'post'} for t in [0,1,2]]}}
            body = json.dumps(scenario,sort_keys=True).encode()
            members = {'native/model.json':native,'native/scenarios/baseline.json':body}
            index = {'model_id':model['model_id'],'model_version':'1.0.0',
                'native_execution':{'model':'native/model.json','scenarios':{'baseline':'native/scenarios/baseline.json'}},
                'members':{name:hashlib.sha256(raw).hexdigest() for name,raw in members.items()}}
            members['index.json']=json.dumps(index).encode();packed=io.BytesIO()
            with zipfile.ZipFile(packed,'w',compression=zipfile.ZIP_DEFLATED) as archive:
                for name,raw in members.items():archive.writestr(name,raw)
            archive_bytes=packed.getvalue();archive_hash=hashlib.sha256(archive_bytes).hexdigest()
            # Both outcomes concern this intentionally mismatched synthetic transfer system.
            evidence=json.dumps({'model_sha256':hashlib.sha256(native).hexdigest(),'request_sha256':hashlib.sha256(body).hexdigest(),
                'analytical_central':'exp(-2*t)','analytical_peripheral':'(1-exp(-2*t))/2',
                'closed_transfer_residual':'-transfer*central','initial_residual_mg_per_h':-1},sort_keys=True).encode()
            axes=['numerical_parity','source_fidelity','conservation','author_code_parity','clinical_validation','physical_dimensions']
            assessments=[{'axis':axis,'outcome':'not_assessed','scope':'No assessment evidence declared.','evidence':None,'findings':[]} for axis in axes]
            assessments[0].update(outcome='passed',scope='Synthetic baseline analytical values at 0, 1 and 2 h only.',
                evidence={'artifact_sha256':hashlib.sha256(evidence).hexdigest(),'method':'Closed-form synthetic trajectory','scope':'The exact native fixture and its stored baseline request.'})
            assessments[2].update(outcome='discrepancy_observed',scope='The claimed closed transfer does not preserve the sum of the two states.',
                evidence={'artifact_sha256':hashlib.sha256(evidence).hexdigest(),'method':'Algebraic sum of synthetic right-hand sides','scope':'Native default transfer=1/h and initial central=1 mg.'},
                findings=[{'id':'missing_transfer_return','description':'Central loses twice the amount transferred to peripheral.',
                    'expression':'d(central + peripheral)/dt = -transfer * central','value':-1,'declared_unit':'mg/h','native_symbols':['transfer','central','peripheral']}])
            manifest={'schema':'qspquest.scientific-assessments/v1','model_id':model['model_id'],'model_version':'1.0.0',
                'archive_sha256':archive_hash,'native_model_sha256':hashlib.sha256(native).hexdigest(),
                'assessments':assessments,'scope':'Synthetic status-separation fixture. No biological or clinical qualification.'}
            catalog={'models':[{'id':model['model_id'],'version':'1.0.0','name':'Synthetic flow assessment',
                'candidate':False,'license':'user supplied','archive':'model.omex','omex_sha256':archive_hash,
                'scenario_ids':['baseline'],'fit_ready':False,'fitting_supported':False}]}
            Path(scratch.name,'model.omex').write_bytes(archive_bytes)
            def write_assessment():
                raw=json.dumps(manifest,indent=2,allow_nan=False).encode()
                catalog['models'][0]['scientific_assessments']={'path':'SCIENTIFIC_ASSESSMENTS.json','sha256':hashlib.sha256(raw).hexdigest()}
                Path(scratch.name,'SCIENTIFIC_ASSESSMENTS.json').write_bytes(raw)
                Path(scratch.name,'catalog.json').write_text(json.dumps(catalog));return raw
            def restart_scientific():
                nonlocal process,responses
                process.terminate();process.wait(timeout=5);process,responses=start()
                rpc('initialize',{'protocolVersion':'2025-11-25','capabilities':{},'clientInfo':{'name':'Scientific assessment example','version':'1'}})
                rpc('notifications/initialized',{},notification=True)
            raw=write_assessment();restart_scientific()
            card=tool('get_model',{'model_id':model['model_id']});declaration=card['scientific_assessment']
            assert declaration=={'file_sha256':hashlib.sha256(raw).hexdigest(),'manifest':manifest}
            lookup={'model_id':model['model_id'],'version':'1.0.0','scenario_id':'baseline',
                'expected_inspection_hash':card['inspection']['context']['inspection_hash']}
            values_checked=refusals=0
            for label,rate in [('baseline',1),('modified',2)]:
                args={'model':{'scenario':lookup},'outputs':['central_amount','peripheral_amount'],'save_csv':label+'.csv'}
                if rate==2:args['parameters']={'transfer':{'value':rate,'unit':'1/h'}}
                planned=tool('plan_run',{'operation':'simulate','arguments':args})
                plan=planned.get('plan') or full_resource(planned['full_plan_resource'])
                receipt=plan['qualification']['scientific_assessment']
                assert receipt['declaration']==declaration and receipt['current_engine_and_request']=='not_assessed'
                assert receipt['request_matches_stored_preset']==(rate==1)
                assert receipt['declaration']['manifest']['assessments'][4]['outcome']=='not_assessed'
                args['expected_plan_hash']=planned['plan_hash'];result=tool('simulate',args)
                complete=full_resource(result['full_result_resource']);assert complete['scientific_assessment']==receipt
                columns={o['name']:o['values'] for o in complete['outputs']}
                for i,t in enumerate(complete['times']):
                    central=math.exp(-2*rate*t);peripheral=(1-central)/2
                    assert abs(columns['central_amount'][i]-central)<1e-8
                    assert abs(columns['peripheral_amount'][i]-peripheral)<1e-8;values_checked+=2
                csv=Path(result['saved_csv']['path']);table=read_table(csv)
                assert table['manifest']['scientific_assessment']==receipt
                sidecar=Path(str(csv)+'.manifest.json');original=sidecar.read_bytes()
                for mutation in ['qualified','validated','clinical','archive','evidence','findings','preset','unknown']:
                    changed=json.loads(original);r=changed['scientific_assessment'];m=r['declaration']['manifest']
                    if mutation=='qualified':r['current_engine_and_request']='qualified'
                    if mutation=='validated':m['assessments'][0]['outcome']='validated'
                    if mutation=='clinical':m['assessments'][4]['outcome']='passed'
                    if mutation=='archive':m['archive_sha256']='0'*64
                    if mutation=='evidence':m['assessments'][0]['evidence']=None
                    if mutation=='findings':m['assessments'][2]['findings']=[]
                    if mutation=='preset':r['request_matches_stored_preset']=not r['request_matches_stored_preset']
                    if mutation=='unknown':r['validated']=True
                    sidecar.write_text(json.dumps(changed))
                    try:read_table(csv)
                    except ValueError:refusals+=1
                    else:raise AssertionError('Scientific receipt tamper accepted: '+mutation)
                    finally:sidecar.write_bytes(original)
            manifest['scope']+=' Revised fixture declaration.';write_assessment();restart_scientific()
            refused=rpc('tools/call',{'name':'plan_run','arguments':{'operation':'simulate','arguments':{'model':{'scenario':lookup}}}})
            assert refused['isError'] and refused['structuredContent']['error']['code']=='inspection_changed'
            assert values_checked==12 and refusals==16
            print('Verified separate scientific axes, 12 analytical values, archived conservation finding, modified-request scope, 16 CSV tamper refusals and changed-declaration freshness.')
            return
        if scheduled_boundaries:
            times = [0, 1, 1.999, 2, 2.001, 3, 4, 4.5, 5]
            count = 0
            for operation in ('<', '<=', '>', '>='):
                example = f"""model scheduled_boundary_example
time_unit = h
parameters
 cutoff = 2 [h] estimated
 k0 = 1 [1/h] fixed
 k1 = 2 [1/h] fixed
switches
 early = time {operation} cutoff
states
 central = 0 [mg]
dynamics
 d/dt(central) = input_central - ifelse(early > 0, k0, k1) * central
dosing
 central [mg] = 1
outputs
 amount = central [mg]
 predicate = early [1]
end"""
                observations = [{'time': {'value':t,'unit':'h'}, 'side':side} for t in times for side in ('pre','post')]
                administrations = [{'target':'central','time':{'value':t,'unit':'h'},'amount':{'value':amount,'unit':'mg'},'delivery':delivery,'bioavailability':1}
                    for t,amount,delivery in [(0,10,{'kind':'bolus'}),(2,5,{'kind':'bolus'}),
                        (3,1,{'kind':'infusion','span':{'kind':'duration','duration':{'value':1,'unit':'h'}}})]]
                run = {'schema':'qspquest.run/v0.1','solver':'diffsol_bdf',
                    'budgets':{'solver_callbacks':1000000,'events':100,'output_values':100000},
                    'regimen':{'start':{'value':0,'unit':'h'},'end':{'value':5,'unit':'h'},'rtol':1e-10,'atol':1e-12,
                        'samples':[],'observations':observations,'administrations':administrations,'checkpoints':[{'value':4.5,'unit':'h'}]}}
                args = {'model':{'source':example},'request':run,'outputs':['amount','predicate'],'save_csv':'boundaries-'+str(count)+'.csv'}
                planned = tool('plan_run',{'operation':'simulate','arguments':args})
                plan = planned.get('plan') or full_resource(planned['full_plan_resource'])
                receipt = plan['scheduled_boundary_receipt']
                assert receipt['stage']=='prepared' and receipt['achieved_error_estimate'] is None
                assert [b['time'] for b in receipt['boundaries']]==[0,2,3,4,4.5]
                assert receipt['fixed_switches'][0]['threshold']==2
                assert receipt['fixed_switches'][0]['parameter_bindings'][0]['name']=='cutoff'
                assert receipt['boundaries'][1]['fixed_switch_names']==['early']
                assert receipt['boundaries'][3]['events'][0]['role']=='infusion_end'
                assert receipt['boundaries'][4]['requested_checkpoint']
                args['expected_plan_hash']=planned['plan_hash']
                result=tool('simulate',args); full=full_resource(result['full_result_resource'])
                executed={**receipt,'stage':'executed'}
                assert full['scheduled_boundary_receipt']==executed
                table=read_table(result['saved_csv']['path']); assert table['manifest']['scheduled_boundary_receipt']==executed
                before=1 if operation.startswith('<') else 2; after=3-before
                values={o['name']:o['values'] for o in full['outputs']}
                for i,(t,side) in enumerate(zip(full['times'],full['sides'])):
                    at2=10*math.exp(-2*before)
                    if t==0: expected=0 if side=='pre' else 10
                    elif t<2: expected=10*math.exp(-before*t)
                    elif t==2: expected=at2+(0 if side=='pre' else 5)
                    else:
                        expected=(at2+5)*math.exp(-after*(t-2))
                        if t>3: expected+=(-math.expm1(-after*(min(t,4)-3))/after)*math.exp(-after*max(0,t-4))
                    predicate={'<':t<2,'<=':t<=2,'>':t>2,'>=':t>=2}[operation]
                    assert abs(values['amount'][i]-expected)<2e-7,(operation,t,side,values['amount'][i],expected)
                    assert values['predicate'][i]==int(predicate)
                csv=Path(result['saved_csv']['path']); sidecar=Path(str(csv)+'.manifest.json'); original=sidecar.read_bytes()
                for mutation in ('threshold','phase','pointwise','identity','observation','achieved_error'):
                    corrupted=json.loads(original); r=corrupted['scheduled_boundary_receipt']
                    if mutation=='threshold': r['fixed_switches'][0]['threshold']=1
                    if mutation=='phase': r['boundaries'][1]['post_binding_phase_indices']=[0]
                    if mutation=='pointwise': r['fixed_switches'][0]['pointwise_value_at_threshold']=1-r['fixed_switches'][0]['pointwise_value_at_threshold']
                    if mutation=='identity': r['planned_execution_identity']['bound_value_hash']='changed'
                    if mutation=='observation': r['observations'][0]['side']='post'
                    if mutation=='achieved_error': r['achieved_error_estimate']=1e-10
                    sidecar.write_text(json.dumps(corrupted))
                    try: read_table(csv)
                    except ValueError: pass
                    else: raise AssertionError('Reader accepted changed boundary '+mutation)
                    finally: sidecar.write_bytes(original)
                count+=1
            print('Verified four predicate conventions, 144 analytical values, native schedule receipts, CSV sides and 24 changed-receipt refusals')
            return
        if initialization:
            model = tool('validate_model', {'source': source})['model']
            native = json.dumps(model, sort_keys=True, separators=(',', ':'), allow_nan=False).encode()
            scenario = {'schema': 'qspquest.run/v0.1', 'solver': 'diffsol_bdf',
                'budgets': {'solver_callbacks': 1000000, 'events': 100, 'output_values': 100000},
                'regimen': {'start': {'value': -20, 'unit': 'h'}, 'end': {'value': 2, 'unit': 'h'},
                    'samples': [], 'rtol': 1e-10, 'atol': 1e-12,
                    'administrations': [{'target': 'marker', 'amount': {'value': 1, 'unit': 'mg'},
                        'time': {'value': 0, 'unit': 'h'}, 'delivery': {'kind': 'bolus'}, 'bioavailability': 1}],
                    'observations': [{'time': {'value': t, 'unit': 'h'}, 'side': 'post'}
                        for t in (-20, -10, -1, 0, 1, 2)]}}
            body = json.dumps(scenario, sort_keys=True).encode()
            members = {'native/model.json': native, 'native/scenarios/baseline.json': body}
            index = {'model_id': model['model_id'], 'model_version': '1.0.0',
                'native_execution': {'model': 'native/model.json',
                    'scenarios': {'baseline': 'native/scenarios/baseline.json'}},
                'members': {name: hashlib.sha256(data).hexdigest() for name, data in members.items()}}
            members['index.json'] = json.dumps(index).encode()
            packed = io.BytesIO()
            with zipfile.ZipFile(packed, 'w', compression=zipfile.ZIP_DEFLATED) as archive:
                for name, data in members.items(): archive.writestr(name, data)
            archive_bytes = packed.getvalue()
            archive_hash = hashlib.sha256(archive_bytes).hexdigest()
            manifest = {'schema': 'qspquest.protocol-initialization/v1', 'model_id': model['model_id'],
                'model_version': '1.0.0', 'archive_sha256': archive_hash,
                'native_model_sha256': hashlib.sha256(native).hexdigest(),
                'scenarios': {'baseline': {'native_request_sha256': hashlib.sha256(body).hexdigest(),
                    'integration_start': {'value': -20, 'unit': 'h'},
                    'intervention_time': {'value': 0, 'unit': 'h'}, 'explicit_initial_override_count': 0,
                    'interpretation': 'Initialize target and marker at zero at -20 h; marker dose at 0 h. No convergence assertion.'}},
                'normalizations': [
                    {'canonical_output': 'fixed_pct', 'numerator_state': 'target', 'denominator_input': 'reference',
                        'multiplier': 100, 'basis': 'declared_reference_input',
                        'interpretation': 'Use the declared reference input, including any explicit override; do not rebaseline.'},
                    {'canonical_output': 'condition_pct', 'numerator_state': 'target', 'denominator_input': 'condition_reference',
                        'multiplier': 100, 'basis': 'condition_specific_binding',
                        'interpretation': 'Use synthesis/clearance evaluated in each native input phase; do not use a trajectory sample.'}],
                'scope': 'Synthetic initialization and explicit input normalization mechanics, not a biological qualification.'}
            catalog = {'models': [{'id': model['model_id'], 'version': '1.0.0', 'name': 'Synthetic initialization',
                'candidate': False, 'license': 'user supplied', 'archive': 'model.omex', 'omex_sha256': archive_hash,
                'scenario_ids': ['baseline'], 'fit_ready': False, 'fitting_supported': False}]}
            Path(scratch.name, 'model.omex').write_bytes(archive_bytes)
            def publish_declaration():
                raw = json.dumps(manifest, indent=2, allow_nan=False).encode()
                catalog['models'][0]['protocol_initialization'] = {'path': 'PROTOCOL_INITIALIZATION.json',
                    'sha256': hashlib.sha256(raw).hexdigest()}
                Path(scratch.name, 'PROTOCOL_INITIALIZATION.json').write_bytes(raw)
                Path(scratch.name, 'catalog.json').write_text(json.dumps(catalog))
                return raw
            def restart():
                nonlocal process, responses
                process.terminate(); process.wait(timeout=5)
                process, responses = start()
                rpc('initialize', {'protocolVersion': '2025-11-25', 'capabilities': {},
                    'clientInfo': {'name': 'Initialization example', 'version': '1'}})
                rpc('notifications/initialized', {}, notification=True)
            raw = publish_declaration(); restart()
            card = tool('get_model', {'model_id': model['model_id']})
            declaration = card['protocol_initialization']
            assert declaration == {'file_sha256': hashlib.sha256(raw).hexdigest(), 'manifest': manifest}
            lookup = {'model_id': model['model_id'], 'version': '1.0.0', 'scenario_id': 'baseline',
                'expected_inspection_hash': card['inspection']['context']['inspection_hash']}
            for name, override in [('baseline', {}), ('predose', {'parameters': {'clearance': {'value': 2, 'unit': '1/h'}}}),
                    ('during-predose', {'parameter_changes': [{'name': 'clearance', 'at': {'value': -10, 'unit': 'h'},
                        'value': {'value': 2, 'unit': '1/h'}}]}),
                    ('at-dose', {'parameter_changes': [{'name': 'clearance', 'at': {'value': 0, 'unit': 'h'},
                        'value': {'value': 2, 'unit': '1/h'}}]})]:
                args = {'model': {'scenario': lookup}, 'outputs': ['fixed_pct', 'condition_pct'],
                    'save_csv': name + '.csv', **override}
                planned = tool('plan_run', {'operation': 'simulate', 'arguments': args})
                plan = planned.get('plan') or full_resource(planned['full_plan_resource'])
                prepared = plan['initialization_receipt']
                assert prepared['stage'] == 'prepared' and prepared['source_declaration'] == declaration
                assert prepared['integration_start'] == -20 and prepared['first_dose_time'] == 0
                assert prepared['predose_integration'] == {'start': -20, 'end': 0}
                assert {s['name']: s['value'] for s in prepared['initialized_states']} == {'target': 0, 'marker': 0}
                phases = prepared['binding_phases']
                timed = name in ('at-dose', 'during-predose')
                boundary = -10 if name == 'during-predose' else 0
                assert [(p['time'], p['activation']) for p in phases] == ([(-20, 'initial'), (boundary, 'post_boundary')] if timed else [(-20, 'initial')])
                assert [next(i['value'] for i in p['inputs'] if i['name'] == 'clearance') for p in phases] == ([1, 2] if timed else [2 if name == 'predose' else 1])
                args['expected_plan_hash'] = planned['plan_hash']
                result = tool('simulate', args); full = full_resource(result['full_result_resource'])
                receipt = full['initialization_receipt']
                assert receipt == {**prepared, 'stage': 'executed'} and receipt['planned_execution_identity'] == full['identity']
                assert read_table(result['saved_csv']['path'])['manifest']['initialization_receipt'] == receipt
                for output in full['outputs']:
                    for t, value in zip(full['times'], output['values']):
                        rate = 2 if name == 'predose' else 1
                        target = 2 / rate * (-math.expm1(-rate * (t + 20)))
                        if timed and t >= boundary:
                            target = 1 + (2 * (-math.expm1(-(boundary + 20))) - 1) * math.exp(-2 * (t - boundary))
                        denominator = 2 if output['name'] == 'fixed_pct' else (1 if name == 'predose' or (timed and t >= boundary) else 2)
                        assert abs(value - 100 * target / denominator) < 2e-6, (name, t, output['name'], value)
                # The reader must reject a receipt with a fabricated phase denominator or identity.
                csv = Path(result['saved_csv']['path']); sidecar = Path(str(csv) + '.manifest.json')
                original = sidecar.read_bytes(); table_manifest = json.loads(original)
                for mutation in ('denominator', 'identity', 'clock'):
                    corrupted = json.loads(original)
                    if mutation == 'denominator': corrupted['initialization_receipt']['normalization_receipts'][0]['denominator_bindings'][0]['value'] += 1
                    if mutation == 'identity': corrupted['initialization_receipt']['planned_execution_identity']['model_hash'] = 'invented'
                    if mutation == 'clock': corrupted['initialization_receipt']['predose_integration']['end'] = -1
                    sidecar.write_text(json.dumps(corrupted))
                    try: read_table(csv)
                    except ValueError: pass
                    else: raise AssertionError('Reader accepted corrupted initialization ' + mutation)
                sidecar.write_bytes(original)
            args = {'model': {'scenario': lookup}, 'parameters': {'reference': {'value': 0, 'unit': 'mg'}}, 'outputs': ['fixed_pct']}
            refused = rpc('tools/call', {'name': 'plan_run', 'arguments': {'operation': 'simulate', 'arguments': args}})
            assert refused['isError'] and refused['structuredContent']['error']['code'] == 'initialization_contract'
            # An unselected normalized output does not impose its denominator on another readout.
            args['outputs'] = ['condition_pct']
            assert tool('plan_run', {'operation': 'simulate', 'arguments': args})['plan']['initialization_receipt']['normalization_receipts'][0]['declaration']['canonical_output'] == 'condition_pct'
            args.pop('parameters'); args['parameter_changes'] = [{'name': 'clearance',
                'at': {'value': -21, 'unit': 'h'}, 'value': {'value': 2, 'unit': '1/h'}}]
            refused = rpc('tools/call', {'name': 'plan_run', 'arguments': {'operation': 'simulate', 'arguments': args}})
            assert refused['isError'] and 'precedes integration start' in refused['structuredContent']['error']['message']
            manifest['scope'] += ' Explicit clock interpretation revised.'
            publish_declaration(); restart()
            refused = rpc('tools/call', {'name': 'get_scenario_request', 'arguments': lookup})
            assert refused['isError'] and refused['structuredContent']['error']['code'] == 'inspection_changed'
            print('Verified four protocol conditions, 48 analytical values, prepared/executed native initialization receipts, both denominator policies, CSV tamper refusal, invalid denominator/boundary refusal and declaration freshness.')
            return
        if precision:
            validated = tool("validate_model", {"source": source})
            model = validated["model"]
            native = json.dumps(model, sort_keys=True, separators=(",", ":"), allow_nan=False).encode()
            mutable = {p['name']: p['default'] for p in model['parameters'] if not p['fixed']}
            immutable = {p['name']: p['default'] for p in model['parameters'] if p['fixed']}
            assert set(mutable) == {'cl'} and immutable == {'v': {'value': 3.0, 'unit': 'L'}}
            scenario = json.loads(json.dumps(request))
            scenario['parameters'] = mutable
            scenario['regimen']['end'] = {'value': 12, 'unit': 'h'}
            scenario['regimen']['observations'] = [
                {'time': {'value': t, 'unit': 'h'}, 'side': 'post'} for t in range(13)]
            body = json.dumps(scenario, sort_keys=True).encode()
            members = {'native/model.json': native, 'native/scenarios/baseline.json': body}
            index = {'model_id': model['model_id'], 'model_version': '1.0.0',
                     'native_execution': {'model': 'native/model.json',
                                          'scenarios': {'baseline': 'native/scenarios/baseline.json'}},
                     'members': {n: hashlib.sha256(b).hexdigest() for n, b in members.items()}}
            members['index.json'] = json.dumps(index).encode()
            packed = io.BytesIO()
            with zipfile.ZipFile(packed, 'w', compression=zipfile.ZIP_DEFLATED) as archive:
                for name, data in members.items(): archive.writestr(name, data)
            archive_bytes = packed.getvalue()
            archive_hash = hashlib.sha256(archive_bytes).hexdigest()
            choices = {'cell_line_parameters': 'synthetic', 'molecule_conversion': 'synthetic',
                       'half_life_constant': 'synthetic'}
            profiles = []
            for identifier, clearance in [('baseline', .3), ('alternative', .6)]:
                values = {'cl': {'value': clearance, 'unit': 'L/h'}}
                profiles.append({'id': identifier, 'choices': choices, 'scenarios': {'baseline': {
                    'activation_cell_line_group': 1, 'killing_cell_line_group': 1, 'donor_group': 1,
                    'parameters': values, 'parameters_sha256': hashlib.sha256(
                        json.dumps(values, sort_keys=True, separators=(',', ':')).encode()).hexdigest(),
                    'transformations': []}}})
            manifest = {'schema': 'qspquest.precision-profiles/v1', 'model_id': model['model_id'],
                        'model_version': '1.0.0', 'archive_sha256': archive_hash,
                        'native_model_sha256': hashlib.sha256(native).hexdigest(), 'default_profile_id': 'baseline',
                        'source': {'doi': 'synthetic contract fixture', 'locators': []},
                        'immutable_parameters': immutable, 'parameter_count': 2, 'mutable_parameter_count': 1,
                        'native_half_life_definition_names': [], 'profiles': profiles,
                        'hash_encoding': 'SDK declared SHA-256 of sorted compact Python JSON; distinct from native identity.',
                        'scope': 'Synthetic parameter-convention demonstration; no biological source or qualification.'}
            raw = json.dumps(manifest, indent=2).encode()
            catalog = {'models': [{'id': model['model_id'], 'version': '1.0.0', 'name': 'Synthetic conventions',
                'candidate': True, 'license': 'user supplied', 'archive': 'model.omex',
                'omex_sha256': archive_hash, 'scenario_ids': ['baseline'], 'fit_ready': False,
                'fitting_supported': False, 'precision_profiles': {'path': 'PRECISION_PROFILES.json',
                    'sha256': hashlib.sha256(raw).hexdigest()}}]}
            Path(scratch.name, 'model.omex').write_bytes(archive_bytes)
            Path(scratch.name, 'PRECISION_PROFILES.json').write_bytes(raw)
            Path(scratch.name, 'catalog.json').write_text(json.dumps(catalog))
            process.terminate(); process.wait(timeout=5)
            process, responses = start()
            rpc('initialize', {'protocolVersion': '2025-11-25', 'capabilities': {},
                'clientInfo': {'name': 'Precision profile example', 'version': '1'}})
            rpc('notifications/initialized', {}, notification=True)
            card = tool('get_model', {'model_id': model['model_id']})
            assert [p['id'] for p in card['precision_profiles']['profiles']] == ['baseline', 'alternative']
            assert card['precision_profiles']['profile_file_sha256'] == hashlib.sha256(raw).hexdigest()
            detailed = full_resource(card['full_model_details_resource'])
            assert detailed['precision_profile_manifest'] == manifest
            lookup = {'model_id': model['model_id'], 'version': '1.0.0', 'scenario_id': 'baseline',
                      'precision_profile_id': 'alternative',
                      'expected_inspection_hash': card['inspection']['context']['inspection_hash']}
            selected = tool('get_scenario_request', lookup)
            assert selected['request']['parameters'] == profiles[1]['scenarios']['baseline']['parameters']
            args = {'model': {'scenario': lookup}, 'outputs': ['cp'], 'save_csv': 'profile/alternative.csv'}
            planned = tool('plan_run', {'operation': 'simulate', 'arguments': args})
            plan = planned.get('plan') or full_resource(planned['full_plan_resource'])
            receipt = plan['precision_profile_receipt']
            assert receipt == selected['precision_profile_receipt']
            assert receipt['parameters_status'] == 'matches_profile'
            args['expected_plan_hash'] = planned['plan_hash']
            result = tool('simulate', args)
            full = full_resource(result['full_result_resource'])
            assert full['precision_profile_receipt'] == receipt
            assert full['identity'] == plan['native_preparation']['plan']['prospective_identity']
            assert full['model_provenance']['precision_profile_receipt'] == receipt
            cp = next(o for o in full['outputs'] if o['name'] == 'cp')
            assert len(full['times']) == 13 and cp['unit'] == 'mg/L'
            for t, value in zip(full['times'], cp['values']):
                assert abs(value - 2 * math.exp(-.2 * t)) < 1e-8
            table = read_table(Path(result['saved_csv']['path']))
            assert table['manifest']['model']['provenance']['metadata']['precision_profile_receipt'] == receipt
            changed = json.loads(json.dumps(args))
            changed['model']['scenario']['precision_profile_id'] = 'baseline'
            refused = rpc('tools/call', {'name': 'simulate', 'arguments': changed})
            assert refused['isError'] and refused['structuredContent']['error']['code'] == 'plan_mismatch'
            changed.pop('expected_plan_hash'); changed.pop('save_csv')
            changed['parameters'] = {'cl': {'value': .9, 'unit': 'L/h'}}
            modified = tool('plan_run', {'operation': 'simulate', 'arguments': changed})
            modified_plan = modified.get('plan') or full_resource(modified['full_plan_resource'])
            assert modified_plan['precision_profile_receipt']['parameters_status'] == 'modified'
            changed['model']['scenario']['precision_profile_id'] = 'invented'
            refused = rpc('tools/call', {'name': 'simulate', 'arguments': changed})
            assert refused['isError'] and refused['structuredContent']['error']['code'] == 'precision_profile_unknown'
            fixed_change = json.loads(json.dumps(args))
            fixed_change.pop('expected_plan_hash'); fixed_change.pop('save_csv')
            fixed_change['parameters'] = {'v': {'value': 4, 'unit': 'L'}}
            refused = rpc('tools/call', {'name': 'plan_run', 'arguments': {
                'operation': 'simulate', 'arguments': fixed_change}})
            assert refused['isError'] and refused['structuredContent']['error']['code'] == 'precision_profile_immutable'
            comparison_args = {'model': {'scenario': {k: v for k, v in lookup.items() if k != 'precision_profile_id'}},
                'outputs': ['cp'], 'end': {'value': 12, 'unit': 'h'},
                'times': [{'value': t, 'unit': 'h'} for t in range(13)],
                'regimens': [{'name': identifier, 'precision_profile_id': identifier,
                    'doses': [{'target': 'central', 'amount': {'value': 6, 'unit': 'mg'},
                               'at': {'value': 0, 'unit': 'h'}}]} for identifier in ['baseline', 'alternative']]}
            comparison_plan = tool('plan_run', {'operation': 'compare_regimens', 'arguments': comparison_args})
            comparison_args['expected_plan_hash'] = comparison_plan['plan_hash']
            comparison_result = tool('compare_regimens', comparison_args)
            for member in comparison_result['regimens']:
                full_member = full_resource(member['result']['full_result_resource'])
                assert full_member['precision_profile_receipt']['profile_id'] == member['name']
                assert full_member['precision_profile_receipt']['parameters_status'] == 'matches_profile'
                column = next(o for o in full_member['outputs'] if o['name'] == 'cp')
                rate = .1 if member['name'] == 'baseline' else .2
                for t, side, value in zip(full_member['times'], full_member['sides'], column['values']):
                    expected = 0 if t == 0 and side == 'pre' else 2 * math.exp(-rate * t)
                    assert abs(value - expected) < 1e-8
            # Re-pin a changed synthetic companion; the previous inspection must be refused.
            values = manifest['profiles'][1]['scenarios']['baseline']['parameters']
            values['cl']['value'] = .7
            manifest['profiles'][1]['scenarios']['baseline']['parameters_sha256'] = hashlib.sha256(
                json.dumps(values, sort_keys=True, separators=(',', ':')).encode()).hexdigest()
            raw = json.dumps(manifest, indent=2).encode()
            catalog['models'][0]['precision_profiles']['sha256'] = hashlib.sha256(raw).hexdigest()
            Path(scratch.name, 'PRECISION_PROFILES.json').write_bytes(raw)
            Path(scratch.name, 'catalog.json').write_text(json.dumps(catalog))
            process.terminate(); process.wait(timeout=5)
            process, responses = start()
            rpc('initialize', {'protocolVersion': '2025-11-25', 'capabilities': {},
                'clientInfo': {'name': 'Changed precision profile example', 'version': '1'}})
            rpc('notifications/initialized', {}, notification=True)
            refused = rpc('tools/call', {'name': 'get_scenario_request', 'arguments': lookup})
            assert refused['isError'] and refused['structuredContent']['error']['code'] == 'inspection_changed'
            print('Verified complete profile discovery/selection; 13 analytical values; native plan/result and CSV receipts; changed plan refused; modified profile labelled; two separately computed comparison profiles.')
            return
        if source_families:
            # The family declaration describes this synthetic fixture only.
            documents = {}
            for model_id in ("family_full", "family_reduced"):
                model_source = source.replace("synthetic_resource_example", model_id)
                if model_id == "family_full":
                    model_source = model_source.replace("central = 0 [mg]", "central = 0 [mg]\n peripheral = 3 [mg]").replace("d/dt(central) =", "d/dt(peripheral) = -cl / v * peripheral\n d/dt(central) =").replace("cp = central / v [mg/L]", "cp = central / v [mg/L]\n peripheral_cp = peripheral / v [mg/L]")
                documents[model_id] = tool("validate_model", {"source": model_source})["model"]
            process.terminate()
            process.wait(timeout=5)
            declaration = {"family_id": "synthetic-pk-family", "paper_id": "synthetic-tutorial-source",
                           "canonical_model": {"model_id": "family_full", "version": "1"}}
            catalog = {"models": []}
            for model_id, fidelity in (("family_full", "full"), ("family_reduced", "reduced"), ("family_metadata", "unrecorded")):
                metadata = {"id": model_id, "version": "1", "license": "synthetic example", "scenario_ids": [],
                            "model_fidelity": fidelity, "source_family": declaration}
                if model_id == "family_metadata":
                    metadata["format"] = "metadata_only"
                else:
                    model_bytes = json.dumps(documents[model_id]).encode("utf-8")
                    index = {"model_id": model_id, "model_version": "1", "native_execution": {"model": "native/model.json"},
                             "members": {"native/model.json": hashlib.sha256(model_bytes).hexdigest()}}
                    buffer = io.BytesIO()
                    with zipfile.ZipFile(buffer, "w", compression=zipfile.ZIP_DEFLATED) as archive:
                        archive.writestr("index.json", json.dumps(index))
                        archive.writestr("native/model.json", model_bytes)
                    raw = buffer.getvalue()
                    filename = model_id + ".omex"
                    Path(scratch.name, filename).write_bytes(raw)
                    metadata.update(archive=filename, omex_sha256=hashlib.sha256(raw).hexdigest())
                catalog["models"].append(metadata)
            Path(scratch.name, "catalog.json").write_text(json.dumps(catalog), encoding="utf-8")
            process, responses = start()
            rpc("initialize", {"protocolVersion": "2025-11-25", "capabilities": {}, "clientInfo": {"name": "Source family example", "version": "1"}})
            rpc("notifications/initialized", {}, notification=True)
            search = tool("search_models", {})
            assert search["contract"]["identity"]["scope"] == "metadata"
            rows = {row["id"]: row for row in search["models"]}
            assert set(rows) == {"family_full", "family_reduced", "family_metadata"}
            for model_id, row in rows.items():
                descriptor = row["discovery"]
                available = model_id != "family_metadata"
                assert row["execution_available"] is available and descriptor["execution_available"] is available
                assert descriptor["executable_backend"] == ("qspquest_native" if available else None)
                assert descriptor["scenario_count"] == row["scenario_count"] == 0
                assert descriptor["source_family"]["declaration"] == declaration
                assert descriptor["source_family"]["member_count"] == 3 and descriptor["source_family"]["members_complete"]
                card = tool("get_model", {"model_id": model_id, "version": "1"})
                assert card["discovery"] == descriptor
                uri = descriptor["source_family"]["full_family_resource"]
                complete = full_text(uri)
                assert hashlib.sha256(complete.encode("utf-8")).hexdigest() == uri.rsplit("/", 1)[1]
                assert json.loads(complete)["declaration"] == declaration and len(json.loads(complete)["members"]) == 3
            exact = tool("search_models", {"model_id": "family_full", "version": "1"})
            assert exact["total"] == 1 and exact["models"][0]["id"] == "family_full"
            assert exact["models"][0]["discovery"]["fidelity"] == "full"
            assert exact["models"][0]["discovery"]["archive_sha256"] == catalog["models"][0]["omex_sha256"]
            assert tool("search_models", {"model_id": "family_full", "version": "missing"})["total"] == 0
            listed = {entry["uri"] for entry in rpc("resources/list", {})["resources"]}
            assert uri in listed and "qspquest://schema/model-discovery" in listed and "qspquest://schema/source-family" in listed
            print("Verified full, reduced and metadata-only records, exact model/version lookup, declared canonical relationship and complete family resource digest. Synthetic catalog declarations; scientific qualification remains unassessed.")
            return
        if artifacts:
            assert capabilities["deployment"]["local_result_artifacts_enabled"]
            assert capabilities["limits"]["local_artifact_retention_seconds"] == 86400
            by_name = {item["name"]: item for item in tools}
            for name in ("simulate", "get_patient_parameters", "compare_regimens"):
                assert by_name[name]["annotations"]["readOnlyHint"] is False
                assert by_name[name]["annotations"]["idempotentHint"] is False
            assert by_name["get_model"]["annotations"]["readOnlyHint"] is True
            # A small input grid and many selected outputs produce a complete result over 4 MB.
            large_source = source.replace("cp = central / v [mg/L]", "\n".join(f"cp{index:02d} = central / v [mg/L]" for index in range(48)))
            large_request = json.loads(json.dumps(request))
            large_request["budgets"]["output_values"] = 1_000_000
            large_request["regimen"]["observations"] = [{"time": {"value": 0, "unit": "h"}, "side": "pre"}] + [
                {"time": {"value": index / 200, "unit": "h"}, "side": "post"} for index in range(6001)]
            arguments = {"model": {"source": large_source}, "request": large_request, "outputs": [f"cp{index:02d}" for index in range(48)]}
            planned = tool("plan_run", {"operation": "simulate", "arguments": arguments})
            native = (planned["plan"] or full_resource(planned["full_plan_resource"]))["native_preparation"]["plan"]
            arguments["expected_plan_hash"] = planned["plan_hash"]
            result = tool("simulate", arguments)
            descriptor = result["result_artifact"]
            assert descriptor["bytes"] > 4_000_000, descriptor
            assert result["full_result_resource"] == descriptor["uri"]
            assert result["contract"]["completeness"] == "resource_linked"
            assert descriptor["uri"] in result["contract"]["resources"]
            assert result["identity"] == native["prospective_identity"]
            full = full_resource(descriptor["uri"])
            assert full["identity"] == result["identity"] and full["plan_hash"] == planned["plan_hash"]
            assert full["effective_request_hash"] == planned["effective_request_hash"]
            assert full["effective_request"]["regimen"]["observations"] == large_request["regimen"]["observations"]
            assert len(full["times"]) == 6002 and len(full["outputs"]) == 48
            assert full["times"][:2] == [0, 0] and full["sides"][:2] == ["pre", "post"]
            for output in full["outputs"]:
                assert output["unit"] == "mg/L" and len(output["values"]) == 6002
                for time, side, value in zip(full["times"], full["sides"], output["values"]):
                    expected = 0 if time == 0 and side == "pre" else 2 * math.exp(-0.1 * time)
                    assert abs(value - expected) < 2e-7, (time, side, value, expected)
            # New calculations neither evict the retained result nor renew its expiry.
            for _ in range(9):
                tool("get_patient_parameters", {"model": {"source": source}, "covariates": {}, "outputs": ["cp"]})
            assert json.loads(read(descriptor["uri"])) == descriptor
            process.terminate()
            process.wait(timeout=5)
            process, responses = start()
            rpc("initialize", {"protocolVersion": "2025-11-25", "capabilities": {}, "clientInfo": {"name": "Retained result example", "version": "1"}})
            rpc("notifications/initialized", {}, notification=True)
            assert descriptor["uri"] in {entry["uri"] for entry in rpc("resources/list", {})["resources"]}
            assert json.loads(read(descriptor["uri"])) == descriptor
            assert full_resource(descriptor["uri"]) == full
            print(f"Verified {descriptor['bytes']} retained bytes, 6002 sided rows, 48 outputs, every analytic value, identity, digest and unchanged expiry after nine further calculations and server restart. Synthetic numerical illustration; scientific qualification remains unassessed.")
            return
        if documentation:
            index = full_resource("qspquest://documentation")
            assert index["schema"] == "qspquest.documentation/v1"
            assert index["package_version"] == capabilities["server_version"]
            assert index["capabilities_uri"] == "qspquest://capabilities"
            catalog = full_resource(index["capability_catalog_uri"])
            assert catalog["schema"] == "qspquest.capability-catalog/v1"
            assert catalog["package_version"] == index["package_version"]
            assert all(entry["evidence_status"] == "untested" for entry in catalog["entries"])
            registry = full_resource(capabilities["capability_registry_uri"])
            assert registry["schema"] == "qspquest.capability-registry/v1"
            assert registry["target"] == capabilities["target"]
            assert registry["catalog_sha256"] == hashlib.sha256(full_text(index["capability_catalog_uri"]).encode()).hexdigest()
            card = tool("get_model", {"model_id": "synthetic_resource_example"})
            query = {"key": {"operation": "simulate", "interface": "mcp", "estimator": "none",
                             "solver": "diffsol_bdf", "target": capabilities["target"],
                             "execution_mode": "local_stdio"},
                     "model": {"model_id": "synthetic_resource_example", "version": card["metadata"]["version"]},
                     "scenario_id": "default"}
            answer = tool(capabilities["capability_query_tool"], query)
            assert answer["declared"] and answer["active_exposed"]
            assert answer["assessment"] == "requires_request_validation" and answer["executable"] is None
            assert not answer["qualified"] and answer["evidence_status"] == "not_assessed"
            assert answer["model"]["archive_sha256"] == card["archive_sha256"]
            listed = {entry["uri"]: entry for entry in rpc("resources/list", {})["resources"]}
            assert all(full_text(uri) for uri in index["topics"].values())
            links = set()
            for entry in index["resources"]:
                assert listed[entry["uri"]]["mimeType"] == entry["mime_type"]
                content = full_text(entry["uri"])
                assert hashlib.sha256(content.encode()).hexdigest() == entry["content_sha256"]
                links.update(re.findall(r'\]\((qspquest://[^\s)]+)\)', content))
            for uri in sorted(links):
                assert full_text(uri)
            example = index["examples"][0]
            model_text = full_text(example["model_uri"])
            example_run = full_resource(example["request_uri"])
            result = tool(example["operation"], {"model": {"source": model_text}, "request": example_run})
            assert len(result["times"]) == 6 and result["time_unit"] == "h"
            assert result["identity"]["compiler_source_sha256"] == catalog["compiler_source_sha256"]
            column = next(output for output in result["outputs"] if output["name"] == "cp")
            assert column["unit"] == "mg/L" and result["sides"] == ["post"] * 6
            for time, value in zip(result["times"], column["values"]):
                assert abs(value - 2 * math.exp(-0.1 * time)) < 1e-8
            print(f"Verified {len(index['resources'])} offline resources, {len(links)} navigable links, capability query and six analytical samples.")
            return
        if linear_pk_doses:
            samples = 0
            for depot, target, infusion in ((True, "depot", False), (True, "central", False),
                                           (False, "central", False), (False, "central", True)):
                pk_source = """model synthetic_linear_pk_doses
time_unit = h
parameters
 cl = 0.3 [L/h] fixed
 v = 3 [L] fixed
 ka = 1.2 [1/h] fixed
 dose_scale = 2 [1] fixed
linear_pk
 amount_unit = mg
 clearance = cl
 compartment central volume v = 0 [mg]
"""
                if depot:
                    pk_source += " depot depot absorption ka = 0 [mg]\n"
                if infusion:
                    pk_source += "dosing\n central [mg] = dose_scale\n"
                pk_source += "outputs\n cp = central / v [mg/L]\nend"
                dose = {"target": target, "amount": {"value": 6000, "unit": "ug"},
                        "at": {"value": 0, "unit": "h"}, "lag": {"value": 1, "unit": "h"},
                        "every": {"value": 6, "unit": "h"}, "count": 2, "bioavailability": 0.5}
                if infusion:
                    dose["duration"] = {"value": 2, "unit": "h"}
                arguments = {"model": {"source": pk_source}, "outputs": ["cp"], "doses": [dose],
                             "end": {"value": 12, "unit": "h"},
                             "times": [{"value": t, "unit": "h"} for t in range(13)]}
                planned = tool("plan_run", {"operation": "simulate", "arguments": arguments})
                plan = planned["plan"] or full_resource(planned["full_plan_resource"])
                receipts = plan["dose_event_receipts"]
                assert len(receipts) == 2
                scale = {"kind": "symbol", "name": "dose_scale"} if infusion else {"kind": "literal", "value": 1.0}
                for i, receipt in enumerate(receipts):
                    assert receipt["stage"] == "prepared_schedule" and receipt["canonical_target"] == target
                    assert receipt["target_state_unit"] == "mg" and receipt["declared_base_scale"] == scale
                    assert receipt["amount_before_model_scaling"] == {"value": 3.0, "unit": "mg"}
                    assert receipt["time"] == {"value": 1.0 + 6*i, "unit": "h"}
                    assert receipt["duration"] == ({"value": 2.0, "unit": "h"} if infusion else None)
                    assert receipt["delivery"] == ("infusion" if infusion else "bolus")
                arguments["expected_plan_hash"] = planned["plan_hash"]
                result = tool("simulate", arguments)
                full = full_resource(result["full_result_resource"]) if result.get("full_result_resource") else result
                assert full["dose_event_receipts"] == [dict(r, stage="executed_schedule") for r in receipts]
                column = next(c for c in full["outputs"] if c["name"] == "cp")
                for t, side, actual in zip(full["times"], full["sides"], column["values"]):
                    expected = 0.0
                    for at in (1, 7):
                        elapsed = t - at
                        if elapsed < 0 or (elapsed == 0 and side == "pre"):
                            continue
                        if infusion:
                            # 3 mg before scale, scale 2, over 2 h in 3 L: 1 mg/L/h.
                            active = min(elapsed, 2)
                            expected += (1 - math.exp(-0.1*active)) / 0.1 * math.exp(-0.1*max(0, elapsed-2))
                        elif target == "depot":
                            expected += 1.2 / 1.1 * (math.exp(-0.1*elapsed) - math.exp(-1.2*elapsed))
                        else:
                            expected += math.exp(-0.1*elapsed)
                    assert abs(actual - expected) < 1e-6, (depot, target, infusion, t, side, actual, expected)
                    samples += 1
            print(f"Verified four linear PK dose workflows, eight prepared/executed receipts and {samples} analytical concentrations.")
            return
        if dose_events:
            baseline = None
            for amount in ({"value":150,"unit":"mg"}, {"value":150000,"unit":"ug"}, {"value":0.15,"unit":"g"}):
                arguments = {"model":{"source":source}, "outputs":["cp"],
                    "end":{"value":24,"unit":"h"}, "times":[{"value":t,"unit":"h"} for t in (0,12,24)],
                    "doses":[{"target":"central","amount":amount,"at":{"value":0,"unit":"h"},"every":{"value":12,"unit":"h"},"count":2}]}
                planned = tool("plan_run", {"operation":"simulate", "arguments":arguments})
                plan = planned["plan"] or full_resource(planned["full_plan_resource"])
                receipts = plan["dose_event_receipts"]
                assert len(receipts)==2
                for i, receipt in enumerate(receipts):
                    assert receipt["stage"]=="prepared_schedule" and receipt["canonical_target"]=="central"
                    assert receipt["time"]=={"value":12.0*i,"unit":"h"}
                    assert receipt["amount_before_model_scaling"]=={"value":150.0,"unit":"mg"}
                    assert receipt["archive_source_coordinate"] is None
                arguments["expected_plan_hash"] = planned["plan_hash"]
                result = tool("simulate", arguments)
                full = full_resource(result["full_result_resource"]) if result.get("full_result_resource") else result
                assert all(r["stage"]=="executed_schedule" for r in full["dose_event_receipts"])
                if baseline is None:
                    baseline=full
                else:
                    assert baseline["outputs"]==full["outputs"] and baseline["times"]==full["times"] and baseline["sides"]==full["sides"]
                column=next(c for c in full["outputs"] if c["name"]=="cp")
                for t, side, actual in zip(full["times"],full["sides"],column["values"]):
                    expected=sum(50*math.exp(-0.1*(t-at)) for at in (0,12) if at<t or (at==t and side=="post"))
                    assert abs(actual-expected)/max(1,abs(expected)) < 1e-6
            print("Verified three equivalent mass inputs, two 150 mg prepared/executed dose receipts and analytical concentrations. Source coordinates are undeclared for inline models.")
            return
        if required_outputs:
            arguments = {"model": {"source": source}, "request": request,
                         "outputs": ["cp", "synthetic_zero"],
                         "required_outputs": [{"output": "cp"}, {"output": "synthetic_zero"}]}
            planned = tool("plan_run", {"operation": "simulate", "arguments": arguments})
            plan = planned["plan"] or full_resource(planned["full_plan_resource"])
            assert plan["effective_request"] == request
            assert all(r["status"] == "selected_for_computation" for r in plan["required_output_receipts"])
            arguments["expected_plan_hash"] = planned["plan_hash"]
            result = tool("simulate", arguments)
            full = full_resource(result["full_result_resource"])
            assert full["effective_request"] == request
            receipts = full["required_output_receipts"]
            assert len(receipts) == 2 and all(r["status"] == "computed" and r["sample_count"] == 301 for r in receipts)
            columns = {o["name"]: o for o in full["outputs"]}
            assert all(x == 0 for x in columns["synthetic_zero"]["values"])
            for t, x in zip(full["times"], columns["cp"]["values"]):
                assert abs(x - 2 * math.exp(-0.1 * t)) < 1e-7
            for requirement, selection, reason in [({"output": "missing"}, ["cp"], "unknown_output"),
                    ({"output": "cp"}, ["synthetic_zero"], "not_selected"),
                    ({"output": "cp", "analyte": "antibody"}, ["cp"], "analyte_undeclared"),
                    ({"output": "cp", "compartment": "plasma"}, ["cp"], "compartment_undeclared")]:
                response = rpc("tools/call", {"name": "simulate", "arguments": {"model": {"source": source}, "request": request,
                                               "outputs": selection, "required_outputs": [requirement]}})
                assert response.get("isError")
                payload = response["structuredContent"]
                failure = payload["contract"]["failure"]["output_requirement"]
                assert payload["contract"]["status"] == "request_failure" and failure["reason"] == reason
                assert not failure["fulfilled"] and payload.get("outputs") is None and payload.get("run_id") is None
            changed = json.loads(json.dumps(arguments)); changed["required_outputs"].pop()
            response = rpc("tools/call", {"name": "simulate", "arguments": changed})
            assert response.get("isError") and response["structuredContent"]["error"]["code"] == "plan_mismatch"
            print("Verified 301 analytical samples, an explicitly computed zero column, four unfulfilled output requirements and changed-plan refusal.")
            return
        if time_validation:
            shifted = json.loads(json.dumps(request))
            shifted["initial_states"] = {"central": {"value": 6, "unit": "mg"}}
            shifted["regimen"].pop("observations")
            shifted["regimen"].update(start={"value": 2, "unit": "h"}, end={"value": 4, "unit": "h"},
                                      samples=[{"value": minute, "unit": "min"} for minute in range(120, 241)], administrations=[])
            arguments = {"model": {"source": source}, "request": shifted, "outputs": ["cp"]}
            planned = tool("plan_run", {"operation": "simulate", "arguments": arguments})
            plan = planned["plan"] or full_resource(planned["full_plan_resource"])
            assert plan["effective_request"] == shifted
            window = plan["time_validation"][0]
            assert window["start"] == 2 and window["end"] == 4 and window["unit"] == "h"
            assert window["sample_times_checked"] == 121
            arguments["expected_plan_hash"] = planned["plan_hash"]
            result = tool("simulate", arguments)
            full = full_resource(result["full_result_resource"])
            assert full["effective_request"] == shifted and full["time_validation"] == plan["time_validation"]
            output = next(item for item in full["outputs"] if item["name"] == "cp")
            assert len(full["times"]) == 121
            for time, value in zip(full["times"], output["values"]):
                assert abs(value - 2 * math.exp(-0.1 * (time - 2))) < 1e-7
            invalid = []
            for value in (None, "NaN"):
                bad = json.loads(json.dumps(shifted)); bad["regimen"]["end"]["value"] = value
                invalid.append((bad, "arguments.request.regimen.end.value", value))
            for quantity in ({"value": 241, "unit": "min"}, {"value": 240, "unit": "mg"}, {"value": 1e308, "unit": "d"}):
                bad = json.loads(json.dumps(shifted)); bad["regimen"]["samples"][-1] = quantity
                invalid.append((bad, "effective_request.regimen.samples[120]", quantity))
            bad = json.loads(json.dumps(shifted)); bad["regimen"]["checkpoints"] = [{"value": 241, "unit": "min"}]
            invalid.append((bad, "effective_request.regimen.checkpoints[0]", {"value": 241, "unit": "min"}))
            bad = json.loads(json.dumps(shifted)); administration = {"target": "central", "time": {"value": 4, "unit": "h"},
                       "amount": {"value": 1, "unit": "mg"}, "delivery": {"kind": "infusion", "span": {"kind": "duration", "duration": {"value": 1, "unit": "min"}}}, "bioavailability": 1}
            bad["regimen"]["administrations"] = [administration]
            invalid.append((bad, "effective_request.regimen.administrations[0]", administration))
            for bad, field, supplied in invalid:
                response = rpc("tools/call", {"name": "simulate", "arguments": {"model": {"source": source}, "request": bad, "outputs": ["cp"]}})
                assert response.get("isError")
                payload = response["structuredContent"]
                assert payload["contract"]["status"] == "request_failure" and payload["contract"]["identity"]["scope"] == "metadata"
                diagnostic = payload["contract"]["failure"]["input_diagnostic"]
                assert diagnostic["field"] == field and diagnostic["supplied_value"] == supplied
                assert not diagnostic["field_truncated"] and not diagnostic["supplied_value_truncated"]
                assert payload.get("run_id") is None
            print("Verified an unchanged shifted protocol, 121 analytical samples and seven field-level input refusals before integration.")
            return
        if overrides:
            arguments = {"model": {"source": source}, "request": request, "outputs": ["cp"],
                         "parameters": {"cl": {"value": 0.6, "unit": "L/h"},
                                        "source_scale": {"value": 9, "unit": "1"}}}
            planned = tool("plan_run", {"operation": "simulate", "arguments": arguments})
            plan = planned["plan"] or full_resource(planned["full_plan_resource"])
            entries = {entry["canonical_parameter"]: entry for entry in plan["override_receipt"]["entries"]}
            assert entries["cl"]["consumption"] == "dynamic"
            assert entries["cl"]["consumers"]["state_equations"] == ["central"]
            assert entries["cl"]["consumers"]["outputs"] == ["cp"]
            assert entries["source_scale"]["consumption"] == "unused"
            assert "cannot change the simulated trajectory" in entries["source_scale"]["warning"]
            arguments["expected_plan_hash"] = planned["plan_hash"]
            result = tool("simulate", arguments)
            full = full_resource(result["full_result_resource"])
            assert full["override_receipt"] == plan["override_receipt"]
            output = next(item for item in full["outputs"] if item["name"] == "cp")
            for time, value in zip(full["times"], output["values"]):
                assert abs(value - 2 * math.exp(-0.2 * time)) < 1e-7
            changed = dict(arguments)
            changed["parameters"] = {"cl": {"value": 0.6, "unit": "L/h"}, "source_scale": {"value": 99, "unit": "1"}}
            refused = rpc("tools/call", {"name": "simulate", "arguments": changed})
            assert refused.get("isError") and refused["structuredContent"]["error"]["code"] == "plan_mismatch"
            del changed["expected_plan_hash"]
            repeated = tool("simulate", changed)
            repeated = full_resource(repeated["full_result_resource"])
            assert repeated["outputs"] == full["outputs"]
            assert repeated["override_receipt"] != full["override_receipt"]
            print("Verified active and unused override receipts, unchanged inactive trajectories, 301 analytical samples and changed-input refusal.")
            return
        if scan:
            scan_source = source.replace("cl = 0.3 [L/h] fixed", "cl = 0.3 [L/h]")
            arguments = {"model": {"source": scan_source}, "request": request,
                         "parameter": "cl", "values": [{"value": value, "unit": "L/h"} for value in (0.3, 0.6)], "output": "cp"}
            planned = tool("plan_run", {"operation": "scan_parameters", "arguments": arguments})
            plan = planned["plan"] or full_resource(planned["full_plan_resource"])
            arguments["expected_plan_hash"] = planned["plan_hash"]
            result = tool("scan_parameters", arguments)
            assert result["output_mapping"] == plan["output_mapping"] and len(result["output_mapping"]) == 1
            mapping = result["output_mapping"][0]
            assert mapping["requested_output"] == "cp" and mapping["returned_unit"] == "mg/L"
            assert len(result["results"]) == 2
            for clearance, point in zip((0.3, 0.6), result["results"]):
                record = point["result"]
                outputs = {item["name"]: item for item in record["outputs"]}
                column = outputs[mapping["returned_column"]]
                assert column["unit"] == mapping["returned_unit"] and len(column["values"]) == len(record["times"]) == 301
                for time, value in zip(record["times"], column["values"]):
                    assert abs(value - 2 * math.exp(-clearance / 3 * time)) < 2e-8
            print("Verified two parameter-scan trajectories, mapped concentration columns and 602 analytical samples.")
            return
        if patient:
            # The future derivative is undefined; initial outputs remain well defined.
            initial_source = source.replace("central = 0 [mg]", "central = 6 [mg]").replace("cl / v * central", "cl / (v - v) * central")
            arguments = {"model": {"source": initial_source}, "covariates": {"v": {"value": 6000, "unit": "mL"}}, "outputs": ["cp"]}
            planned = tool("plan_run", {"operation": "get_patient_parameters", "arguments": arguments})
            plan = planned["plan"] or full_resource(planned["full_plan_resource"])
            native = plan["native_preparation"]["plan"]
            assert native["schema"] == "qspquest.initial-values-plan/v0.1"
            assert native["states"][0]["value"] == 6 and native["time"] == 0
            arguments["expected_plan_hash"] = planned["plan_hash"]
            result = tool("get_patient_parameters", arguments)
            assert result["patient_quantities"] == {"cp": 1} and result["patient_units"] == {"cp": "mg/L"}
            assert result["identity"] == native["prospective_identity"]
            assert result["contract"]["identity"]["scope"] == "execution"
            assert result["identity"]["algorithm"] == "initial_output_evaluation"
            assert result["effective_initial_values"]["states"][0]["value"] == 6
            assert result["qualification"]["current_engine_and_request"] == "not_assessed"
            arguments["covariates"]["v"]["value"] = 3000
            refused = rpc("tools/call", {"name": "get_patient_parameters", "arguments": arguments})
            assert refused.get("isError") and refused["structuredContent"]["error"]["code"] == "plan_mismatch"
            # A stored scenario's administrations are excluded, while its initial state is retained.
            scenario = tool("get_patient_parameters", {"model": {"scenario": {"model_id": "synthetic_resource_example", "scenario_id": "default"}}, "covariates": {}, "outputs": ["cp"]})
            assert scenario["patient_quantities"] == {"cp": 0}
            assert "regimen" not in scenario["effective_request"]
            print("Verified prepared initial values, converted volume, retained initial exposure, no future derivative or scenario administration, execution identity and changed-input refusal. Synthetic illustration; scientific qualification remains unassessed.")
            return
        if comparison:
            arguments = {"model": {"source": source}, "end": {"value": 30, "unit": "h"},
                         "times": [{"value": i, "unit": "h"} for i in range(31)],
                         "outputs": ["cp"], "regimens": [
                             {"name": name, "doses": [{"target": "central", "at": {"value": 0, "unit": "h"},
                                                      "amount": {"value": amount, "unit": "mg"}}]}
                             for name, amount in [("low", 3), ("high", 6)]]}
            planned = tool("plan_run", {"operation": "compare_regimens", "arguments": arguments})
            plan = planned["plan"] or full_resource(planned["full_plan_resource"])
            assert planned["contract"]["identity"]["scope"] == "metadata"
            members = plan["native_preparation"]["plan"]["regimens"]
            assert [member["name"] for member in members] == ["low", "high"]
            assert all(member["preparation"]["qualification"]["current_engine_and_request"] == "not_assessed" for member in members)
            arguments["expected_plan_hash"] = planned["plan_hash"]
            result = tool("compare_regimens", arguments)
            assert result["plan_hash"] == planned["plan_hash"]
            assert result["effective_request_hash"] == planned["effective_request_hash"]
            for index, (member, record) in enumerate(zip(members, result["regimens"])):
                preparation = member["preparation"]
                full = full_resource(record["result"]["full_result_resource"])
                assert full["identity"] == preparation["native_preparation"]["plan"]["prospective_identity"]
                assert record["result"]["plan_hash"] == preparation["plan_hash"]
                assert full["effective_request_hash"] == preparation["context"]["effective_request_hash"]
                assert len(full["times"]) == 32 and full["sides"][:2] == ["pre", "post"]
                for time, side, value in zip(full["times"], full["sides"], full["outputs"][0]["values"]):
                    expected = 0 if time == 0 and side == "pre" else (index + 1) * math.exp(-0.1 * time)
                    assert abs(value - expected) < 1e-6
            arguments["regimens"][1]["doses"][0]["amount"]["value"] = 7
            refused = rpc("tools/call", {"name": "compare_regimens", "arguments": arguments})
            assert refused.get("isError") and refused["structuredContent"]["error"]["code"] == "plan_mismatch"
            binding_source = """model synthetic_binding_conditions
 time_unit = h
parameters
 kon = 0.5 [1/h] fixed
 start_amount = 6 [mg] fixed
states
 free = start_amount [mg]
 bound = 0 [mg]
dynamics
 d/dt(free) = -kon * free
 d/dt(bound) = kon * free
outputs
 free_amount = free [mg]
 bound_amount = bound [mg]
end"""
            conditions = {"model": {"source": binding_source}, "end": {"value": 2, "unit": "h"},
                          "times": [{"value": i / 4, "unit": "h"} for i in range(9)],
                          "parameters": {"kon": {"value": 0.5, "unit": "1/h"}, "start_amount": {"value": 6, "unit": "mg"}},
                          "initial_states": {"free": {"value": 2, "unit": "mg"}, "bound": {"value": 0, "unit": "mg"}},
                          "outputs": ["free_amount", "bound_amount"], "regimens": [
                              {"name": "A_bind", "doses": []},
                              {"name": "B_zero", "doses": [], "parameters": {"kon": {"value": 0, "unit": "1/h"}},
                               "initial_states": {"free": {"value": 2000, "unit": "ug"}}},
                              {"name": "C_double_start", "doses": [], "initial_states": {"free": {"value": 4, "unit": "mg"}}}]}
            planned = tool("plan_run", {"operation": "compare_regimens", "arguments": conditions})
            plan = planned["plan"] or full_resource(planned["full_plan_resource"])
            members = plan["native_preparation"]["plan"]["regimens"]
            conditions["expected_plan_hash"] = planned["plan_hash"]
            comparison = tool("compare_regimens", conditions)
            for index, (member, record) in enumerate(zip(members, comparison["regimens"])):
                receipt = record["input_receipt"]
                assert receipt == member["input_receipt"] and record["name"] == member["name"]
                full = full_resource(record["result"]["full_result_resource"])
                assert full["identity"] == member["preparation"]["native_preparation"]["plan"]["prospective_identity"]
                assert receipt["native_bound_value_hash"] == full["identity"]["bound_value_hash"]
                assert receipt["effective_request_hash"] == full["effective_request_hash"]
                assert full["effective_request"]["parameters"]["start_amount"] == {"value": 6, "unit": "mg"}
                assert full["effective_request"]["initial_states"]["bound"] == {"value": 0, "unit": "mg"}
                start, rate = (4 if index == 2 else 2), (0 if index == 1 else 0.5)
                outputs = {column["name"]: column for column in full["outputs"]}
                free, bound = outputs["free_amount"], outputs["bound_amount"]
                assert free["unit"] == bound["unit"] == "mg" and len(full["times"]) == 9
                for time, actual_free, actual_bound in zip(full["times"], free["values"], bound["values"]):
                    assert abs(actual_free - start * math.exp(-rate * time)) < 1e-6
                    assert abs(actual_free + actual_bound - start) < 1e-7
            a, b, c = [row["input_receipt"] for row in comparison["regimens"]]
            assert a["requested_inputs_sha256"] != b["requested_inputs_sha256"]
            assert a["native_parameter_contract_sha256"] != b["native_parameter_contract_sha256"]
            assert a["native_parameter_contract_sha256"] == c["native_parameter_contract_sha256"]
            assert a["native_bound_value_hash"] != c["native_bound_value_hash"]
            assert b["requested_inputs"]["initial_states"]["free"] == {"value": 2000, "unit": "ug"}
            conditions["regimens"][1]["parameters"]["kon"]["value"] = 0.1
            refused = rpc("tools/call", {"name": "compare_regimens", "arguments": conditions})
            assert refused.get("isError") and refused["structuredContent"]["error"]["code"] == "plan_mismatch"
            print("Verified two prepared regimens, 64 event-sided analytical samples, three named counterfactual conditions with 54 analytical values, complete input receipts, native parameter/state identities and changed-later-input refusal. Synthetic illustration; scientific qualification remains unassessed.")
            return
        if steady_state:
            if "steady_state" not in capabilities["tools"]:
                raise RuntimeError("Periodic steady state requires an MCP binary built with --features analysis")
            root = Path(__file__).resolve().parents[2]
            model_source = (root / "conformance/models/synthetic-linear-pk.json").read_text()
            periodic_request = json.loads((root / "conformance/requests/synthetic-linear-pk-steady.json").read_text())
            arguments = {"model": {"source": model_source}, "request": periodic_request}
            planned = tool("plan_run", {"operation": "steady_state", "arguments": arguments})
            plan = planned["plan"] or full_resource(planned["full_plan_resource"])
            native = plan["native_preparation"]["plan"]
            assert native["schema"] == "qspquest.periodic-plan/v0.1"
            assert native["maximum_cycle_executions"] == len(native["states"]) + 2
            assert all(state["seed"] is None and state["unit"] == "mg" for state in native["states"])
            assert not native["fixed_point_uniqueness_and_stability_certified"]
            assert "initial_states" not in native
            arguments["expected_plan_hash"] = planned["plan_hash"]
            result = tool("steady_state", arguments)
            assert result["plan_hash"] == planned["plan_hash"]
            assert result["identity"] == native["prospective_identity"]
            assert result["maximum_scaled_residual"] <= 1
            arguments["request"]["relative_tolerance"] = 1e-9
            refused = rpc("tools/call", {"name": "steady_state", "arguments": arguments})
            assert refused.get("isError") and refused["structuredContent"]["error"]["code"] == "plan_mismatch"
            print("Verified periodic preparation, exact execution identity, bounded cycle residual and changed-tolerance refusal. Synthetic illustration; uniqueness, stability and scientific qualification remain unassessed.")
            return
        if morris:
            if "morris_screening" not in {item["name"] for item in tools}:
                raise RuntimeError("Morris requires an MCP binary built with --features analysis")
            analysis_request = json.loads((Path(__file__).resolve().parents[2] / "conformance/requests/synthetic-morris.json").read_text())
            arguments = {"model": {"source": source}, "request": analysis_request}
            planned = tool("plan_run", {"operation": "morris_screening", "arguments": arguments})
            plan = planned["plan"] or full_resource(planned["full_plan_resource"])
            native = plan["native_preparation"]["plan"]
            assert native["design"]["seed"] == 42 and len(native["scan"]["points"]) == 6
            assert native["observation"] == {"time": 4.0, "side": "post"}
            assert native["output"] == {"name": "cp", "unit": "mg/L"}
            assert planned["contract"]["identity"]["scope"] == "metadata" and "effects" not in native
            arguments["expected_plan_hash"] = planned["plan_hash"]
            result = tool("morris_screening", arguments)
            assert result["plan_hash"] == planned["plan_hash"]
            assert result["result"]["analysis_request_hash"] == native["analysis_request_hash"]
            expected_values = {}
            for point, actual in zip(native["scan"]["points"], result["result"]["scan"]["results"]):
                assert point["run"]["prospective_identity"] == actual["result"]["identity"]
                inputs = {item["name"]: item["value"] for item in point["run"]["binding_phases"][0]["inputs"]}
                value = 6 / inputs["v"] * math.exp(-inputs["cl"] / inputs["v"] * 4)
                expected_values[point["id"]] = value
                assert abs(actual["result"]["outputs"][0]["values"][native["row"]] - value) < 2e-7
            for effect in result["result"]["effects"]:
                expected = [(expected_values[step["to_point"]] - expected_values[step["from_point"]]) / step["normalized_delta"]
                            for step in native["steps"] if step["parameter"] == effect["parameter"]]
                assert len(effect["effects"]) == len(expected)
                assert all(abs(a - b) < 2e-7 for a, b in zip(effect["effects"], expected))
            arguments["request"]["problem"]["request"]["seed"] = 43
            refused = rpc("tools/call", {"name": "morris_screening", "arguments": arguments})
            assert refused.get("isError") and refused["structuredContent"]["error"]["code"] == "plan_mismatch"
            print("Verified six seeded Morris points, analytical effects, complete planning identities and changed-design refusal. Synthetic illustration; no scientific qualification.")
            return
        if input_bindings:
            identifier = "synthetic_resource_example"
            card = tool("get_model", {"model_id": identifier})
            digest = card["inspection"]["context"]["inspection_hash"]
            scalars = tool("get_model", {"model_id": identifier, "page": {"section": "scalars", "expected_inspection_hash": digest}})["model_page"]
            defaults = {r["canonical_name"]: r["record"]["data"]["default"] for r in scalars["entries"] if r["record"]["kind"] == "parameter"}
            assert defaults["cl"] == {"value": 0.3, "unit": "L/h"} and defaults["v"] == {"value": 3.0, "unit": "L"}
            arguments = {"model": {"library": {"model_id": identifier, "expected_inspection_hash": digest}},
                         "parameters": {"cl": {"value": 300, "unit": "mL/h"}},
                         "initial_states": {"central": {"value": 6000, "unit": "ug"}},
                         "end": {"value": 1, "unit": "h"}, "times": [{"value": 0, "unit": "h"}, {"value": 1, "unit": "h"}], "outputs": ["cp"]}
            # Unknown names receive a binding diagnostic even without a hash guard.
            unchecked = json.loads(json.dumps(arguments))
            unchecked["model"] = {"library": {"model_id": identifier}}
            for operation, field, group, name in [
                ("simulate", "parameters", "parameter", "retired_cl"),
                ("simulate", "initial_states", "initial_state", "retired_central"),
                ("scan_parameters", "parameter", "scan_parameter", "retired_cl"),
                ("get_patient_parameters", "covariates", "patient_input", "retired_cl"),
            ]:
                rejected_args = json.loads(json.dumps(unchecked))
                if operation in ("get_patient_parameters", "scan_parameters"):
                    rejected_args = {"model": unchecked["model"]}
                if field == "parameter":
                    rejected_args[field] = name
                    rejected_args["values"] = [{"value": 0.3, "unit": "L/h"}]
                else:
                    rejected_args[field] = {name: {"value": 1, "unit": "1"}}
                refused = rpc("tools/call", {"name": "plan_run", "arguments": {"operation": operation, "arguments": rejected_args}})
                payload = refused["structuredContent"]
                assert "input_binding" in payload["contract"]["failure"], (operation, payload)
                binding = payload["contract"]["failure"]["input_binding"]
                assert refused.get("isError") and payload["error"]["code"] == "input_alias_unknown"
                assert binding["group"] == group and binding["supplied_name"] == name
                assert binding["current_inspection_hash"] == digest
                assert binding["inspection_index_resource"] == card["inspection_index_resource"]
                assert binding["migration"] == "inspect_current_bindings"
                assert binding["numerical_execution_started"] is False
                assert "result" not in payload and "plan" not in payload
            planned = tool("plan_run", {"operation": "simulate", "arguments": arguments})
            arguments["expected_plan_hash"] = planned["plan_hash"]
            result = tool("simulate", arguments)
            complete = full_resource(result["full_result_resource"]) if result.get("full_result_resource") else result
            assert complete["outputs"][0]["unit"] == "mg/L"
            assert all(abs(v - 2 * math.exp(-0.1 * t)) < 2e-7 for t, v in zip(complete["times"], complete["outputs"][0]["values"]))
            process.terminate(); process.wait(timeout=5)
            Path(scratch.name, "example.qsp").write_text(source.replace(" v = 3 [L] fixed", " v = 6 [L] fixed"))
            process, responses = start()
            rpc("initialize", {"protocolVersion": "2025-11-25", "capabilities": {}, "clientInfo": {"name": "Binding migration example", "version": "1"}})
            rpc("notifications/initialized", {}, notification=True)
            arguments.pop("expected_plan_hash")
            refused = rpc("tools/call", {"name": "plan_run", "arguments": {"operation": "simulate", "arguments": arguments}})
            change = refused["structuredContent"]["contract"]["failure"]["inspection_change"]
            assert refused.get("isError") and change["expected_inspection_hash"] == digest
            assert change["migration"] == "inspect_current_bindings" and change["current_inspection_hash"] != digest
            current = full_resource(change["inspection_index_resource"])
            assert current["context"]["inspection_hash"] == change["current_inspection_hash"]
            print("Verified native input units/defaults, unknown-name migration across four workflows, inspected request binding, analytical unit conversion and explicit changed-binding migration. Synthetic illustration; no scientific qualification.")
            return
        if model_pages:
            identifier = "synthetic_resource_example"
            card = tool("get_model", {"model_id": identifier})
            index = full_resource(card["inspection_index_resource"])
            assert index == card["inspection"] and index["execution_available"]
            digest = index["context"]["inspection_hash"]
            listed = {r["uri"] for r in rpc("resources/list", {})["resources"]}
            assert index["index_resource"] in listed
            checked = 0
            for section in index["sections"]:
                offset = 0; paths = []
                while True:
                    page = tool("get_model", {"model_id": identifier, "page": {
                        "section": section["section"], "offset": offset, "limit": 1,
                        "expected_inspection_hash": digest}})["model_page"]
                    assert page["context"] == index["context"]
                    assert full_resource(page["page_resource"]) == page
                    for row in page["entries"]:
                        complete = full_resource(row["record_resource"])
                        assert complete["context"] == index["context"] and complete["entry"] == row
                        assert row["record"] is not None
                        paths.append(row["native_path"]); checked += 1
                    if page["next_offset"] is None:
                        assert page["next_page_resource"] is None
                        break
                    next_page = full_resource(page["next_page_resource"])
                    assert next_page["offset"] == page["next_offset"] > offset
                    offset = page["next_offset"]
                assert len(paths) == len(set(paths)) == section["record_count"]
            rejected = rpc("tools/call", {"name": "get_model", "arguments": {
                "model_id": identifier, "page": {"section": "outputs", "limit": 1,
                "expected_inspection_hash": "sha256:" + "0" * 64}}})
            assert rejected.get("isError") and rejected["structuredContent"]["error"]["code"] == "inspection_changed"
            print(f"Verified four inspection sections, {checked} complete native declarations, page/record resource equivalence and changed-content refusal. Synthetic illustration; no scientific qualification.")
            return
        if model_cards:
            card = tool("get_model", {"model_id": "synthetic_resource_example"})
            assert card["detail"] == "compact" and card["time_unit"] == "h"
            assert card["interface"]["parameters"] and card["interface"]["dose_targets"]
            details = tool("get_model", {"model_id": card["metadata"]["id"],
                                         "version": card["metadata"]["version"], "detail": "full"})
            full = full_resource(details["full_model_details_resource"])
            assert full["metadata"]["version"] == card["metadata"]["version"]
            assert full["archive_sha256"] == card["archive_sha256"]
            assert full["source"] == source and full["parameters"] and full["equations"]
            scenario = tool("get_scenario_request", {"model_id": card["metadata"]["id"],
                            "version": card["metadata"]["version"], "scenario_id": "default"})
            assert scenario["request"] == request
            print("Verified compact model card, complete metadata and stored scenario.")
            return
        arguments = {"model": {"source": source}, "request": request, "outputs": ["cp"]}
        if csv_output:
            arguments["save_csv"] = "study/trajectory.csv"
        planned = tool("plan_run", {"operation": "simulate", "arguments": arguments})
        if csv_output:
            assert not (csv_output / "study/trajectory.csv").exists()
            assert not (csv_output / "study/trajectory.csv.manifest.json").exists()
        assert planned["contract"]["identity"]["scope"] == "metadata"
        plan = planned["plan"] or full_resource(planned["full_plan_resource"])
        assert plan["plan_hash"] == planned["plan_hash"]
        assert plan["qualification"]["current_engine_and_request"] == "not_assessed"
        native = plan["native_preparation"]["plan"]
        assert len(native["observations"]) == 301
        assert native["request"]["regimen"]["observations"] == request["regimen"]["observations"]
        assert native["request"]["regimen"]["end"] == request["regimen"]["end"]
        assert native["outputs"] == [{"name": "cp", "unit": "mg/L"}]
        arguments["expected_plan_hash"] = planned["plan_hash"]
        result = tool("simulate", arguments)
        assert result["plan_hash"] == planned["plan_hash"]
        assert result["effective_request_hash"] == planned["effective_request_hash"]
        assert result["identity"] == native["prospective_identity"]
        assert result["trajectory_kind"] == "compact_preview"
        assert result["full_result_resource"]
        assert result["contract"]["completeness"] == "resource_linked"
        assert result["full_result_resource"] in result["contract"]["resources"]
        full = full_resource(result["full_result_resource"])
        assert len(full["times"]) == 301
        assert full["identity"] == result["identity"]
        assert full["plan_hash"] == planned["plan_hash"]
        assert full["effective_request_hash"] == result["effective_request_hash"]
        assert full["effective_request"]["regimen"]["end"] == request["regimen"]["end"]
        mapping = full["output_mapping"]
        assert mapping == plan["output_mapping"] and len(mapping) == 1
        assert mapping[0]["requested_output"] == "cp" and mapping[0]["canonical_output"] == "cp"
        assert mapping[0]["returned_unit"] == "mg/L" and mapping[0]["declared_source_unit"] is None
        returned = {item["name"]: item for item in full["outputs"]}
        output = returned[mapping[0]["returned_column"]]
        assert output["name"] == "cp" and output["unit"] == "mg/L"
        for t, side, value in zip(full["times"], full["sides"], output["values"]):
            expected = 2 * math.exp(-0.1 * t)
            assert side == "post" and abs(value - expected) <= 1e-8 + 1e-4 * abs(expected)
        replay = tool("simulate", {"model": {"source": source},
                                   "request": full["effective_request"], "outputs": ["cp"]})
        replay_full = full_resource(replay["full_result_resource"])
        assert replay_full["effective_request"] == full["effective_request"]
        assert replay_full["effective_request_hash"] == full["effective_request_hash"]
        assert replay_full["outputs"] == full["outputs"]
        # A complete native empty grid stays empty in the effective inputs.
        # The engine's boundary rows remain its own execution behavior.
        empty = json.loads(json.dumps(request))
        empty["regimen"].pop("observations")
        empty_result = tool("simulate", {"model": {"source": source}, "request": empty,
                                         "outputs": ["cp"], "detail": "full"})
        assert empty_result["effective_request"]["regimen"]["samples"] == []
        assert "observations" not in empty_result["effective_request"]["regimen"]
        assert empty_result["effective_request_hash"] != full["effective_request_hash"]

        threshold = tool("simulate", {"model": {"source": source},
            "initial_states": {"central": {"value": 30, "unit": "mg"}},
            "end": {"value": 10, "unit": "h"}, "outputs": ["cp"],
            "readouts": [{"kind": "time_above", "output": "cp",
                          "threshold": {"value": 5000, "unit": "ug/L"}}]})
        readout = threshold["readouts"]["results"][0]
        assert readout["threshold"] == 5 and readout["unit"] == "h"
        assert abs(readout["value"] - math.log(2) / 0.1) < 1e-4

        for bad, field in [
            ({"request": "simulate for one day"}, "request"),
            ({"request": {"end": {"value": 10, "unit": "h"}}}, "request"),
            ({"end": {"value": 0, "unit": "h"}}, "end.value"),
            ({"end": {"value": 10, "unit": "h"}, "readouts": [
                {"kind": "time_above", "output": "cp", "threshold": {"value": 5, "unit": "kg"}}]},
             "readouts[0].threshold"),
        ]:
            bad["model"] = {"source": source}
            response = rpc("tools/call", {"name": "simulate", "arguments": bad})
            refused = response["structuredContent"]
            assert response["isError"] and refused["contract"]["status"] == "request_failure"
            assert field in refused["error"]["message"]
            assert "outputs" not in refused and "full_result_resource" not in refused
        print("Verified preserved native grids, four request refusals and a converted threshold against the analytic crossing time.")
        if csv_output:
            target = csv_output / "study" / "trajectory.csv"
            assert Path(result["saved_csv"]["path"]) == target
            original = target.read_bytes()
            bundle = read_table(target)
            manifest = bundle["manifest"]
            assert manifest["producer"] == "mcp" and manifest["qualification"]["qualified"] is False
            assert manifest["inputs"]["modified"] is True
            assert manifest["table"]["sha256"] == result["saved_csv"]["table_sha256"]
            metadata_path = Path(result["saved_csv"]["manifest_path"])
            metadata_bytes = metadata_path.read_bytes()
            assert hashlib.sha256(metadata_bytes).hexdigest() == result["saved_csv"]["manifest_sha256"]
            assert manifest["engine_identity"] == full["identity"]
            assert manifest["inputs"]["effective_request"] == full["effective_request"]
            assert [column["unit"] for column in manifest["table"]["columns"]] == ["h", None, "mg/L"]
            rows = bundle["rows"]
            assert len(rows) == 301 and list(rows[0]) == ["time_h", "side", "cp"]
            for row in rows:
                expected = 2 * math.exp(-0.1 * float(row["time_h"]))
                assert row["side"] == "post" and abs(float(row["cp"]) - expected) <= 1e-8 + 1e-4 * abs(expected)
            repeated = rpc("tools/call", {"name": "simulate", "arguments": arguments})
            assert repeated.get("isError") is True and target.read_bytes() == original and metadata_path.read_bytes() == metadata_bytes
            print(f"Verified 301 CSV rows, units, inputs and both preserved bundle files: {target}")
        print(f"Verified 301 observations; final concentration {output['values'][-1]:.8f} mg/L")
    finally:
        process.terminate()
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait()
        scratch.cleanup()


if __name__ == "__main__":
    main()
