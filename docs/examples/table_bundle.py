"""Reimport a CSV analysis bundle after checking its bytes, rows and event sides.

Run: python3 docs/examples/table_bundle.py path/to/trajectory.csv
The digest detects corruption; it does not authenticate or qualify a producer.
"""
import csv
from decimal import Decimal, InvalidOperation
import hashlib
import io
import json
import math
from pathlib import Path
import re
import sys

MAX_CSV = 256 * 1024 * 1024
MAX_MANIFEST = 16 * 1024 * 1024
MAX_ROWS = 2_000_000
SCHEMA = 'qspquest.table-manifest/v1'
RULE = 'nondecreasing_time;equal_time_requires_pre_then_post'


def _keys(value, names, label):
    if not isinstance(value, dict) or set(value) != set(names.split()):
        raise ValueError(label + ' has missing or unknown fields')


def _unique(pairs):
    obj = {}
    for key, value in pairs:
        if key in obj:
            raise ValueError('Duplicate manifest key: ' + key)
        obj[key] = value
    return obj


def _text(value):
    return value is None or (isinstance(value, str) and bool(value) and '\x00' not in value)


def _object_or_null(value):
    return value is None or isinstance(value, dict)


def _read(path, maximum):
    if path.is_symlink() or not path.is_file():
        raise ValueError('Bundle requires ordinary CSV and manifest files')
    with path.open('rb') as stream:
        raw = stream.read(maximum + 1)
    if len(raw) > maximum:
        raise ValueError('Bundle file exceeds its supported size limit')
    return raw


def _nonfinite(value):
    raise ValueError('Nonfinite JSON number: ' + value)


def _receipts(manifest):
    columns = {c['name']: c for c in manifest['table']['columns']}
    def text(value):
        return isinstance(value, str) and bool(value.strip()) and len(value) <= 512 and not any(ord(c) < 32 for c in value)
    def quantity(value, positive=False):
        _keys(value, 'value unit', 'Dose quantity')
        if type(value['value']) not in (int, float) or not math.isfinite(value['value']) or not text(value['unit']) or (positive and value['value'] < 0):
            raise ValueError('Invalid dose quantity')
    for key in ('required_output_receipts', 'dose_event_receipts'):
        if key in manifest and (manifest['producer'] != 'mcp' or not isinstance(manifest[key], list)):
            raise ValueError('Receipt lists require the MCP producer')
    for receipt in manifest.get('required_output_receipts', []):
        _keys(receipt, 'requirement canonical_output returned_column returned_unit declared_analyte declared_compartment status sample_count', 'Output receipt')
        _keys(receipt['requirement'], 'output analyte compartment', 'Output requirement')
        requirement = receipt['requirement']
        if not text(requirement['output']) or not all(text(receipt[k]) for k in ('canonical_output', 'returned_column', 'returned_unit')):
            raise ValueError('Invalid output receipt labels')
        for requested, declared in [('analyte', 'declared_analyte'), ('compartment', 'declared_compartment')]:
            if receipt[declared] is not None and not text(receipt[declared]):
                raise ValueError('Invalid output identity')
            if requirement[requested] is not None and (not text(requirement[requested]) or requirement[requested] != receipt[declared]):
                raise ValueError('Unfulfilled output identity')
        column = columns.get(receipt['returned_column'])
        if column is None and receipt['returned_column'] == receipt['canonical_output']:
            matches = [c for c in columns.values() if c['native_name'] == receipt['canonical_output']]
            column = matches[0] if len(matches) == 1 else None
        if receipt['status'] != 'computed' or type(receipt['sample_count']) is not int or receipt['sample_count'] != manifest['table']['rows'] or column is None or column['native_name'] != receipt['canonical_output'] or column['unit'] != receipt['returned_unit']:
            raise ValueError('Output receipt differs from table')
    indices = set()
    for receipt in manifest.get('dose_event_receipts', []):
        _keys(receipt, 'stage event_index canonical_target delivery time amount_before_model_scaling duration target_state_unit declared_base_scale archive_source_coordinate notice', 'Dose receipt')
        if receipt['stage'] != 'executed_schedule' or receipt['delivery'] not in ('bolus', 'infusion') or type(receipt['event_index']) is not int or receipt['event_index'] < 0 or receipt['event_index'] in indices:
            raise ValueError('Invalid executed dose receipt')
        indices.add(receipt['event_index'])
        if not all(text(receipt[k]) for k in ('canonical_target', 'target_state_unit', 'notice')):
            raise ValueError('Invalid dose labels')
        quantity(receipt['time']); quantity(receipt['amount_before_model_scaling'], True)
        if receipt['time']['unit'] != manifest['table']['columns'][0]['unit']:
            raise ValueError('Dose time unit differs from table')
        if receipt['delivery'] == 'infusion':
            quantity(receipt['duration'], True)
            if receipt['duration']['value'] <= 0 or receipt['duration']['unit'] != receipt['time']['unit']:
                raise ValueError('Invalid infusion duration')
        elif receipt['duration'] is not None:
            raise ValueError('Bolus receipt cannot carry a duration')
        source = receipt['archive_source_coordinate']
        if source is not None:
            _keys(source, 'native_target amount_unit state_unit source_dose_unit source_coordinate scaling', 'Source dose coordinate')
            if not all(text(v) for v in source.values()) or source['native_target'] != receipt['canonical_target'] or source['state_unit'] != receipt['target_state_unit'] or source['amount_unit'] != receipt['amount_before_model_scaling']['unit']:
                raise ValueError('Source coordinate differs from dose event')
        nodes = [receipt['declared_base_scale']]
        operators = {'unary': ('positive', 'negative', 'not'), 'binary': ('add', 'subtract', 'multiply', 'divide', 'power', 'modulo'), 'compare': ('lt', 'le', 'gt', 'ge', 'eq', 'ne'), 'boolean': ('and', 'or')}
        while nodes:
            node = nodes.pop()
            if not isinstance(node, dict):
                raise ValueError('Invalid dose scale expression')
            kind = node.get('kind')
            if kind == 'literal':
                _keys(node, 'kind value', 'Scale literal')
                if type(node['value']) not in (bool, int, float) or (type(node['value']) is not bool and not math.isfinite(node['value'])):
                    raise ValueError('Invalid scale literal')
                continue
            if kind == 'symbol':
                _keys(node, 'kind name', 'Scale symbol')
                if not text(node['name']):
                    raise ValueError('Invalid scale symbol')
                continue
            if kind in operators:
                _keys(node, 'kind operator arguments', 'Scale operator')
                if node['operator'] not in operators[kind]:
                    raise ValueError('Invalid scale operator')
            elif kind == 'call':
                _keys(node, 'kind name arguments', 'Scale call')
                if node['name'] not in ('exp','log','log10','sqrt','sin','cos','tan','tanh','abs','min','max','floor','ceil','hill','ifelse'):
                    raise ValueError('Invalid scale function')
            elif kind == 'conditional':
                _keys(node, 'kind arguments', 'Scale conditional')
            else:
                raise ValueError('Invalid scale expression kind')
            args = node['arguments']; arity = {'unary':1,'binary':2,'compare':2,'conditional':3}.get(kind)
            if not isinstance(args, list) or (arity is not None and len(args) != arity):
                raise ValueError('Invalid scale arity')
            nodes.extend(args)


def _initialization_receipt(manifest):
    """Check initialization transport, native phase consistency and identity."""
    receipt = manifest.get('initialization_receipt')
    if receipt is None:
        return
    def keys(value, names):
        _keys(value, names, 'Initialization receipt')
    def text(value):
        return isinstance(value, str) and bool(value.strip()) and len(value) <= 1024 and not any(ord(c) < 32 for c in value)
    def number(value):
        if type(value) not in (int, float) or not math.isfinite(value):
            raise ValueError('Initialization coordinates must be finite numbers')
        return value
    def unit(value):
        keys(value, 'dimensions scale_numerator scale_denominator declared')
        if not isinstance(value['dimensions'], list) or len(value['dimensions']) != 8 or any(type(n) is not int or not -32768 <= n <= 32767 for n in value['dimensions']):
            raise ValueError('Invalid native unit dimensions')
        if any(not isinstance(value[k], str) or not re.fullmatch('[1-9][0-9]*', value[k]) for k in ('scale_numerator', 'scale_denominator')) or (value['declared'] is not None and not text(value['declared'])):
            raise ValueError('Invalid exact native unit scale')
    def normalization(value):
        keys(value, 'canonical_output numerator_state denominator_input multiplier basis interpretation')
        if any(not text(value[k]) for k in ('canonical_output', 'numerator_state', 'denominator_input', 'interpretation')) or number(value['multiplier']) <= 0 or value['basis'] not in ('declared_reference_input', 'condition_specific_binding'):
            raise ValueError('Invalid normalization policy')
    keys(receipt, 'schema stage time_unit integration_start integration_end first_dose_time predose_integration initialized_states binding_phases source_declaration declared_scenario normalization_receipts planned_execution_identity notice')
    start, end = number(receipt['integration_start']), number(receipt['integration_end'])
    if manifest['producer'] != 'mcp' or receipt['schema'] != 'qspquest.initialization-receipt/v1' or receipt['stage'] != 'executed' or receipt['time_unit'] != manifest['table']['columns'][0]['unit'] or end < start or not text(receipt['notice']) or receipt['planned_execution_identity'] != manifest['engine_identity']:
        raise ValueError('Initialization receipt clock, producer or execution identity differs from table')
    first = receipt['first_dose_time']
    if first is not None:
        number(first)
        if not start <= first <= end:
            raise ValueError('First dose is outside initialization clock')
    expected_window = {'start': start, 'end': first} if first is not None and first > start else None
    if receipt['predose_integration'] != expected_window:
        raise ValueError('Pre-dose interval differs from native clock')
    if not isinstance(receipt['initialized_states'], list) or not isinstance(receipt['binding_phases'], list) or not receipt['binding_phases'] or not isinstance(receipt['normalization_receipts'], list):
        raise ValueError('Initialization requires states, input phases and normalization lists')
    states = set()
    for state in receipt['initialized_states']:
        keys(state, 'name unit value'); number(state['value'])
        if not text(state['name']) or not text(state['unit']) or state['name'] in states:
            raise ValueError('Invalid or duplicate initialized state')
        states.add(state['name'])
    phases = []
    previous = start
    for index, phase in enumerate(receipt['binding_phases']):
        keys(phase, 'time activation inputs'); time = number(phase['time'])
        if phase['activation'] != ('initial' if index == 0 else 'post_boundary') or not previous <= time <= end or (index == 0 and time != start) or not isinstance(phase['inputs'], list):
            raise ValueError('Invalid native binding phase order or activation')
        previous = time
        inputs = {}
        for item in phase['inputs']:
            keys(item, 'name kind value unit'); number(item['value']); unit(item['unit'])
            if not text(item['name']) or item['name'] in inputs or item['kind'] not in ('parameter', 'covariate', 'individual_parameter', 'derived_covariate'):
                raise ValueError('Invalid or duplicate native phase input')
            inputs[item['name']] = item
        phases.append((phase, inputs))
    source = receipt['source_declaration']
    declarations = {}
    if source is not None:
        keys(source, 'file_sha256 manifest')
        declared = source['manifest']
        keys(declared, 'schema model_id model_version archive_sha256 native_model_sha256 scenarios normalizations scope')
        for digest in (source['file_sha256'], declared['archive_sha256'], declared['native_model_sha256']):
            if not isinstance(digest, str) or not re.fullmatch('[0-9a-f]{64}', digest):
                raise ValueError('Invalid source initialization digest')
        if declared['schema'] != 'qspquest.protocol-initialization/v1' or declared['model_id'] != manifest['model']['id'] or declared['model_version'] != manifest['model']['version'] or declared['archive_sha256'] != manifest['model']['archive_sha256'] or not text(declared['scope']) or not isinstance(declared['scenarios'], dict) or not isinstance(declared['normalizations'], list):
            raise ValueError('Initialization declaration differs from exported model')
        for scenario in declared['scenarios'].values():
            keys(scenario, 'native_request_sha256 integration_start intervention_time explicit_initial_override_count interpretation')
            if not isinstance(scenario['native_request_sha256'], str) or not re.fullmatch('[0-9a-f]{64}', scenario['native_request_sha256']) or type(scenario['explicit_initial_override_count']) is not int or scenario['explicit_initial_override_count'] < 0 or not text(scenario['interpretation']):
                raise ValueError('Invalid declared scenario initialization')
            for field in ('integration_start', 'intervention_time'):
                quantity = scenario[field]
                if quantity is None and field == 'intervention_time':
                    continue
                keys(quantity, 'value unit'); number(quantity['value'])
                if not text(quantity['unit']):
                    raise ValueError('Invalid declared scenario clock unit')
        for declaration in declared['normalizations']:
            normalization(declaration)
            if declaration['canonical_output'] in declarations:
                raise ValueError('Duplicate source normalization policy')
            declarations[declaration['canonical_output']] = declaration
    if receipt['declared_scenario'] is not None and (source is None or receipt['declared_scenario'] not in source['manifest']['scenarios'] or receipt['declared_scenario'] != manifest['inputs']['scenario_id']):
        raise ValueError('Initialization scenario differs from effective inputs')
    outputs = {c['native_name'] or c['name'] for c in manifest['table']['columns'] if c['kind'] == 'output'}
    expected = {name for name in declarations if name in outputs}
    seen = set()
    for normalized in receipt['normalization_receipts']:
        keys(normalized, 'declaration denominator_bindings')
        declaration = normalized['declaration']; normalization(declaration)
        name = declaration['canonical_output']
        if name in seen or name not in expected or declaration != declarations[name] or declaration['numerator_state'] not in states or not isinstance(normalized['denominator_bindings'], list) or len(normalized['denominator_bindings']) != len(phases):
            raise ValueError('Normalization differs from source, selected outputs or native phases')
        seen.add(name)
        for binding, (phase, inputs) in zip(normalized['denominator_bindings'], phases):
            keys(binding, 'time activation value unit'); number(binding['value']); unit(binding['unit'])
            item = inputs.get(declaration['denominator_input'])
            kinds = ('parameter', 'covariate') if declaration['basis'] == 'declared_reference_input' else ('individual_parameter',)
            if item is None or item['kind'] not in kinds or binding != {'time': phase['time'], 'activation': phase['activation'], 'value': item['value'], 'unit': item['unit']} or binding['value'] <= 0:
                raise ValueError('Normalization denominator differs from its actual native input phase')
    if seen != expected:
        raise ValueError('Selected normalized output lacks an initialization receipt')


def _scheduled_boundary_receipt(manifest, rows=None):
    """Check schedule transport against initialization, identity and CSV observation sides."""
    receipt = manifest.get('scheduled_boundary_receipt')
    if receipt is None:
        return
    def keys(value, names):
        _keys(value, names, 'Scheduled boundary receipt')
    def number(value):
        if type(value) not in (int, float) or not math.isfinite(value):
            raise ValueError('Boundary coordinates must be finite numbers')
        return value
    def index(value, count):
        if type(value) is not int or not 0 <= value < count:
            raise ValueError('Invalid boundary or phase index')
        return value
    def expression(root):
        nodes = [root]
        for node in nodes:
            if not isinstance(node, dict) or len(nodes) > 10000:
                raise ValueError('Invalid boundary expression')
            kind = node.get('kind')
            if kind == 'literal':
                keys(node, 'kind value')
                if type(node['value']) is not bool: number(node['value'])
            elif kind == 'symbol':
                keys(node, 'kind name')
                if not _text(node['name']) or node['name'] is None: raise ValueError('Invalid boundary symbol')
            else:
                field = 'name' if kind == 'call' else 'operator'
                keys(node, 'kind arguments' + ('' if kind == 'conditional' else ' ' + field))
                options = {'unary':('positive','negative','not'), 'binary':('add','subtract','multiply','divide','power','modulo'),
                           'compare':('lt','le','gt','ge','eq','ne'), 'boolean':('and','or'),
                           'call':('exp','log','log10','sqrt','sin','cos','tan','tanh','abs','min','max','floor','ceil','hill','ifelse')}
                if kind != 'conditional' and node.get(field) not in options.get(kind,()): raise ValueError('Invalid boundary operator')
                args = node['arguments']; arity = {'unary':1,'binary':2,'compare':2,'conditional':3}.get(kind)
                if not isinstance(args,list) or (arity is not None and len(args)!=arity): raise ValueError('Invalid boundary expression arity')
                nodes.extend(args)
    keys(receipt, 'schema stage time_unit integration_start integration_end planned_execution_identity fixed_switches dose_relative_switches boundaries observations observation_driver_rows pointwise_input_overrides requested_solver requested_relative_tolerance requested_state_absolute_tolerances achieved_error_estimate observation_convention')
    initial = manifest.get('initialization_receipt')
    if not isinstance(initial,dict) or receipt['schema'] != 'qspquest.scheduled-boundaries/v1' or receipt['stage'] != 'executed':
        raise ValueError('Boundary receipt requires an executed initialization receipt')
    if receipt['planned_execution_identity'] != manifest['engine_identity'] or any(receipt[k]!=initial[k] for k in ('planned_execution_identity','time_unit','integration_start','integration_end')):
        raise ValueError('Boundary clock or identity differs from native initialization')
    if receipt['achieved_error_estimate'] is not None or not _text(receipt['observation_convention']) or receipt['observation_convention'] is None:
        raise ValueError('Boundary receipt cannot assert an achieved error estimate')
    if receipt['requested_solver'] not in ('diffsol_bdf','diffsol_tsit45','diffsol_esdirk34','diffsol_tr_bdf2','diffsol_rosenbrock23','diffsol_rodas5p','analytic_linear_pk') or number(receipt['requested_relative_tolerance'])<=0:
        raise ValueError('Invalid requested solver or tolerance')
    for field in ('fixed_switches','dose_relative_switches','boundaries','observations','observation_driver_rows','pointwise_input_overrides','requested_state_absolute_tolerances'):
        if not isinstance(receipt[field],list): raise ValueError('Boundary receipt inventories must be arrays')
    boundaries = receipt['boundaries']; phases = initial['binding_phases']; seen_phases = set(); names = set()
    native_inputs = {v['name']:v for v in phases[0]['inputs']}
    prior = None
    for boundary in boundaries:
        keys(boundary, 'time native_checkpoint requested_checkpoint post_binding_phase_indices fixed_switch_names events')
        t = number(boundary['time'])
        if not receipt['integration_start']<=t<=receipt['integration_end'] or (prior is not None and t<=prior): raise ValueError('Boundary times must be ordered inside the run')
        prior=t
        if any(type(boundary[k]) is not bool for k in ('native_checkpoint','requested_checkpoint')) or (boundary['requested_checkpoint'] and not boundary['native_checkpoint']): raise ValueError('Invalid checkpoint origin')
        for field in ('post_binding_phase_indices','fixed_switch_names','events'):
            if not isinstance(boundary[field],list): raise ValueError('Invalid boundary inventory')
        for i in boundary['post_binding_phase_indices']:
            index(i,len(phases))
            if i==0 or i in seen_phases or phases[i]['time']!=t or phases[i]['activation']!='post_boundary': raise ValueError('Boundary phase differs from initialization')
            seen_phases.add(i)
        if len(boundary['fixed_switch_names'])!=len(set(boundary['fixed_switch_names'])): raise ValueError('Duplicate boundary switch')
        for event in boundary['events']:
            keys(event, 'native_event_index role target')
            if type(event['native_event_index']) is not int or event['native_event_index']<0 or event['role'] not in ('reset','bolus','infusion_start','infusion_end') or not _text(event['target']) or event['target'] is None:
                raise ValueError('Invalid native event reference')
    if seen_phases!=set(range(1,len(phases))): raise ValueError('Boundary table omits a native post-boundary phase')
    for switch in receipt['fixed_switches']:
        keys(switch, 'name time_expression parameter_bindings threshold comparison pointwise_value_at_threshold initial_input_value boundary_index')
        name=switch['name']; threshold=number(switch['threshold']); comparison=switch['comparison']
        if name in names or name not in native_inputs or number(switch['initial_input_value'])!=native_inputs[name]['value'] or comparison not in (None,'lt','le','gt','ge'):
            raise ValueError('Invalid declared native fixed switch')
        names.add(name); expression(switch['time_expression'])
        expected = None if comparison is None else int(comparison in ('le','ge'))
        if switch['pointwise_value_at_threshold'] != expected: raise ValueError('Pointwise predicate value differs from comparison')
        if not isinstance(switch['parameter_bindings'],list): raise ValueError('Invalid threshold parameter bindings')
        binding_names=set()
        for binding in switch['parameter_bindings']:
            if not isinstance(binding,dict) or binding.get('name') in binding_names or binding!=native_inputs.get(binding.get('name')) or binding.get('kind')!='parameter': raise ValueError('Threshold binding differs from native initialization')
            binding_names.add(binding['name'])
        if switch['boundary_index'] is not None:
            b=boundaries[index(switch['boundary_index'],len(boundaries))]
            if b['time']!=threshold or name not in b['fixed_switch_names']: raise ValueError('Fixed switch boundary correspondence differs')
        if comparison is not None and receipt['integration_start']<=threshold<=receipt['integration_end'] and switch['boundary_index'] is None: raise ValueError('In-window predicate boundary is missing')
    if {n for b in boundaries for n in b['fixed_switch_names']}!=names.intersection({n for s in receipt['fixed_switches'] if s['boundary_index'] is not None for n in [s['name']]}): raise ValueError('Undeclared boundary switch')
    for switch in receipt['dose_relative_switches']:
        keys(switch, 'name target offset comparison simultaneous'); expression(switch['offset'])
        if not _text(switch['name']) or switch['name'] is None or not _text(switch['target']) or switch['target'] is None or switch['name'] in names or switch['comparison'] not in ('lt','le','gt','ge') or switch['simultaneous'] not in ('first','last'): raise ValueError('Invalid dose-relative declaration')
        names.add(switch['name'])
    observations=receipt['observations']
    if len(observations)!=manifest['table']['rows'] or len(receipt['observation_driver_rows'])!=len(observations): raise ValueError('Boundary observation inventory differs from CSV')
    for observation,i in zip(observations,receipt['observation_driver_rows']):
        keys(observation, 'time side');t=number(observation['time'])
        if not receipt['integration_start']<=t<=receipt['integration_end'] or observation['side'] not in ('pre','post') or type(i) is not int or i<0: raise ValueError('Invalid native observation')
    seen_overrides=set()
    for override in receipt['pointwise_input_overrides']:
        keys(override, 'observation covariates');i=index(override['observation'],len(observations))
        if i in seen_overrides or not isinstance(override['covariates'],dict) or not override['covariates']: raise ValueError('Invalid pointwise override inventory')
        seen_overrides.add(i)
        for name,value in override['covariates'].items():
            keys(value, 'value unit')
            if name not in names or value['value'] not in (0,1) or type(value['value']) is bool or value['unit']!='1': raise ValueError('Invalid pointwise predicate override')
            fixed=next((s for s in receipt['fixed_switches'] if s['name']==name),None)
            if fixed and (observations[i]['time']!=fixed['threshold'] or value['value']!=fixed['pointwise_value_at_threshold']): raise ValueError('Pointwise override differs from fixed predicate')
    tolerances={}
    for tolerance in receipt['requested_state_absolute_tolerances']:
        keys(tolerance, 'state value unit')
        if tolerance['state'] in tolerances or number(tolerance['value'])<=0: raise ValueError('Invalid native state tolerance')
        tolerances[tolerance['state']]=tolerance['unit']
    if tolerances!={s['name']:s['unit'] for s in initial['initialized_states']}: raise ValueError('Tolerance coordinates differ from native initialized states')
    if rows is not None:
        columns=manifest['table']['columns'];time_name=columns[0]['name'];side_name=columns[1]['name']
        if any(row[time_name]!=o['time'] or row[side_name]!=o['side'] for row,o in zip(rows,observations)): raise ValueError('CSV observations differ from scheduled boundary receipt')


def _scientific_assessment(manifest):
    """Check declared status provenance and scope; never certify scientific truth."""
    receipt = manifest.get('scientific_assessment')
    if receipt is None:
        return
    def keys(v, names):
        if not isinstance(v, dict) or set(v) != set(names.split()):
            raise ValueError('Scientific assessment has missing or unknown fields')
    def text(v):
        if not isinstance(v, str) or not v.strip() or len(v.encode()) > 1024 or any(ord(c)<32 or 127<=ord(c)<=159 for c in v):
            raise ValueError('Invalid scientific assessment text')
    def digest(v):
        if not isinstance(v, str) or re.fullmatch('[0-9a-f]{64}', v) is None:
            raise ValueError('Invalid scientific assessment SHA-256')
    keys(receipt, 'schema declaration effective_source_matches_archive request_matches_stored_preset current_engine_and_request notice')
    if receipt['schema'] != 'qspquest.mcp-scientific-assessment/v1' or receipt['current_engine_and_request'] != 'not_assessed':
        raise ValueError('Scientific declaration cannot qualify the current engine and request')
    text(receipt['notice'])
    for k in ['effective_source_matches_archive', 'request_matches_stored_preset']:
        if receipt[k] is not None and type(receipt[k]) is not bool:
            raise ValueError('Invalid scientific assessment applicability')
    declaration = receipt['declaration']
    if declaration is None:
        if receipt['effective_source_matches_archive'] is not None:
            raise ValueError('Undeclared archive cannot have scientific assessment applicability')
        return
    keys(declaration, 'file_sha256 manifest');digest(declaration['file_sha256'])
    m = declaration['manifest']
    keys(m, 'schema model_id model_version archive_sha256 native_model_sha256 assessments scope')
    if m['schema'] != 'qspquest.scientific-assessments/v1':
        raise ValueError('Unsupported scientific assessment declaration')
    digest(m['archive_sha256']);digest(m['native_model_sha256']);text(m['model_id']);text(m['model_version']);text(m['scope'])
    model = manifest['model']
    if (m['model_id'],m['model_version'],m['archive_sha256']) != (model['id'],model['version'],model['archive_sha256']):
        raise ValueError('Scientific declaration differs from table model identity')
    metadata = (model.get('provenance') or {}).get('metadata') or {}
    descriptor = metadata.get('scientific_assessments') or {}
    if declaration != metadata.get('scientific_assessment_contract') or declaration['file_sha256'] != descriptor.get('sha256'):
        raise ValueError('Scientific declaration differs from retained pinned model provenance')
    axes = ['numerical_parity','source_fidelity','conservation','author_code_parity','clinical_validation','physical_dimensions']
    if not isinstance(m['assessments'],list) or len(m['assessments']) != 6 or {a.get('axis') for a in m['assessments'] if isinstance(a,dict)} != set(axes):
        raise ValueError('Scientific assessment must retain every independent axis exactly once')
    for a in m['assessments']:
        keys(a, 'axis outcome scope evidence findings');text(a['scope'])
        if a['outcome'] not in ('not_assessed','passed','failed','discrepancy_observed') or not isinstance(a['findings'],list) or len(a['findings'])>8:
            raise ValueError('Invalid scientific assessment outcome or findings')
        if a['outcome']=='not_assessed':
            if a['evidence'] is not None or a['findings']:
                raise ValueError('Unassessed axes cannot carry assessment evidence')
        else:
            keys(a['evidence'], 'artifact_sha256 method scope');digest(a['evidence']['artifact_sha256']);text(a['evidence']['method']);text(a['evidence']['scope'])
        if a['outcome'] in ('failed','discrepancy_observed') and not a['findings']:
            raise ValueError('Failed or discrepant assessment lacks findings')
        ids=set()
        for f in a['findings']:
            keys(f, 'id description expression value declared_unit native_symbols')
            for k in ['id','description','expression','declared_unit']:text(f[k])
            if f['id'] in ids or type(f['value']) not in (int,float) or not math.isfinite(f['value']):
                raise ValueError('Invalid scientific finding value or duplicate id')
            ids.add(f['id']);names=f['native_symbols']
            if not isinstance(names,list) or not 1<=len(names)<=64 or any(not isinstance(n,str) for n in names) or len(set(names)) != len(names):
                raise ValueError('Invalid scientific finding native symbols')
            for n in names:text(n)
    inputs = manifest['inputs']
    if type(receipt['effective_source_matches_archive']) is not bool or (inputs.get('modified') is False and not receipt['effective_source_matches_archive']):
        raise ValueError('Scientific source applicability differs from unchanged table provenance')
    if inputs.get('scenario_id') is not None and receipt['request_matches_stored_preset'] != (inputs.get('modified') is False):
        raise ValueError('Scientific assessment preset scope differs from effective input provenance')

def read_table(path):
    """Return the validated manifest and numerical rows, preserving header names."""
    path = Path(path)
    manifest = json.loads(_read(Path(str(path) + '.manifest.json'), MAX_MANIFEST),
                          object_pairs_hook=_unique, parse_constant=_nonfinite)
    optional = ' '.join(k for k in ('required_output_receipts', 'dose_event_receipts', 'initialization_receipt', 'scheduled_boundary_receipt', 'scientific_assessment') if isinstance(manifest, dict) and k in manifest)
    _keys(manifest, 'schema producer table events model engine_identity inputs qualification ' + optional, 'Manifest')
    if manifest['schema'] != SCHEMA or manifest['producer'] not in ('mcp', 'model_library', 'cli', 'python'):
        raise ValueError('Unsupported table manifest version or producer')
    table, events, model, inputs, qualification = [manifest[k] for k in ('table', 'events', 'model', 'inputs', 'qualification')]
    _keys(table, 'filename sha256 bytes rows columns', 'Table')
    if table['filename'] != path.name or not isinstance(table['sha256'], str) or not re.fullmatch('[0-9a-f]{64}', table['sha256']):
        raise ValueError('Table filename or SHA-256 is invalid')
    for key, limit in [('bytes', MAX_CSV), ('rows', MAX_ROWS)]:
        if type(table[key]) is not int or not 0 <= table[key] <= limit:
            raise ValueError('Invalid table ' + key)
    columns = table['columns']
    if not isinstance(columns, list) or len(columns) < 2:
        raise ValueError('Table requires ordered time and side columns')
    names = []
    for column in columns:
        _keys(column, 'name unit kind native_name meaning', 'Column')
        if not _text(column['name']) or column['name'] is None or any(not _text(column[k]) for k in ('unit', 'native_name', 'meaning')):
            raise ValueError('Column names and units must be nonempty text or null')
        if column['kind'] not in ('time', 'event_side', 'output', 'state', 'source_parameter'):
            raise ValueError('Unsupported column kind')
        names.append(column['name'])
    if len(set(names)) != len(names) or columns[0]['kind'] != 'time' or columns[1]['kind'] != 'event_side' or names[1] != 'side' or any(c['kind'] in ('time', 'event_side') for c in columns[2:]):
        raise ValueError('Ordered columns must be unique, beginning with time and side')
    _keys(events, 'rule pre post event_pairs', 'Events')
    if events['rule'] != RULE or any(type(events[k]) is not int or events[k] < 0 for k in ('pre', 'post', 'event_pairs')):
        raise ValueError('Invalid event rule or counts')
    _keys(model, 'id version archive_sha256 provenance', 'Model')
    if any(not _text(model[k]) for k in ('id', 'version', 'archive_sha256')) or not _object_or_null(model['provenance']) or not _object_or_null(manifest['engine_identity']):
        raise ValueError('Invalid model or engine context')
    if model['archive_sha256'] is not None and not re.fullmatch('[0-9a-f]{64}', model['archive_sha256']):
        raise ValueError('Invalid archive digest')
    _keys(inputs, 'effective_request selected_outputs effective_request_hash scenario_id modified', 'Inputs')
    selected = inputs['selected_outputs']
    if not _object_or_null(inputs['effective_request']) or any(not _text(inputs[k]) for k in ('effective_request_hash', 'scenario_id')) or (inputs['modified'] is not None and type(inputs['modified']) is not bool):
        raise ValueError('Invalid effective input context')
    if selected is not None and (not isinstance(selected, list) or any(not isinstance(s, str) or not s for s in selected) or len(set(selected)) != len(selected)):
        raise ValueError('Selected outputs must be unique names or null')
    _keys(qualification, 'qualified status scope unit_status coordinates recorded_dimensional_qualification recorded_availability', 'Qualification')
    if qualification['qualified'] is not False or qualification['status'] != 'not_assessed' or qualification['scope'] != 'current_engine_and_effective_request':
        raise ValueError('A table manifest cannot assert current-run qualification')
    if qualification['unit_status'] not in ('unqualified', 'source_scale_unqualified') or not _text(qualification['coordinates']):
        raise ValueError('Invalid unit/coordinate status')
    if qualification['coordinates'] == 'unchanged_source_numeric_values' and qualification['unit_status'] != 'source_scale_unqualified':
        raise ValueError('Source coordinates require a source-scale warning')
    if (qualification['recorded_dimensional_qualification'] is not None and type(qualification['recorded_dimensional_qualification']) is not bool) or not _object_or_null(qualification['recorded_availability']):
        raise ValueError('Invalid recorded archive evidence context')
    json.dumps(manifest, allow_nan=False)
    _receipts(manifest)
    _scientific_assessment(manifest)
    _initialization_receipt(manifest)
    _scheduled_boundary_receipt(manifest)
    raw = _read(path, MAX_CSV)
    if len(raw) != table['bytes'] or hashlib.sha256(raw).hexdigest() != table['sha256']:
        raise ValueError('CSV byte count or SHA-256 differs from its manifest')
    reader = csv.reader(io.StringIO(raw.decode('utf-8'), newline=''), strict=True)
    rows = []
    observed = {'pre': 0, 'post': 0, 'event_pairs': 0}
    previous, previous_side, paired = None, None, False
    try:
        if next(reader, None) != names:
            raise ValueError('CSV header differs from the ordered manifest columns')
        for row in reader:
            if len(row) != len(names) or len(rows) >= MAX_ROWS:
                raise ValueError('Invalid CSV row width or count')
            time = Decimal(row[0])
            if not time.is_finite() or row[1] not in ('pre', 'post'):
                raise ValueError('Invalid CSV time or event side')
            if previous is not None:
                if time < previous:
                    raise ValueError('CSV times must be nondecreasing')
                if time == previous:
                    if paired or (previous_side, row[1]) != ('pre', 'post'):
                        raise ValueError('Equal-time CSV rows must be exactly pre then post')
                    paired = True
                    observed['event_pairs'] += 1
                else:
                    paired = False
            previous, previous_side = time, row[1]
            observed[row[1]] += 1
            values = [float(v) if i != 1 else v for i, v in enumerate(row)]
            if any(not math.isfinite(v) for i, v in enumerate(values) if i != 1):
                raise ValueError('CSV numerical values must be finite')
            rows.append(dict(zip(names, values)))
    except (csv.Error, InvalidOperation) as error:
        raise ValueError('Malformed CSV or time: ' + str(error)) from error
    if len(rows) != table['rows'] or any(events[k] != v for k, v in observed.items()):
        raise ValueError('CSV row count or event counts differ from its manifest')
    _scheduled_boundary_receipt(manifest, rows)
    return {'manifest': manifest, 'rows': rows}


if __name__ == '__main__':
    try:
        if len(sys.argv) != 2:
            raise ValueError('Usage: python3 docs/examples/table_bundle.py CSV_PATH')
        print(json.dumps(read_table(sys.argv[1])['manifest'], indent=2, allow_nan=False))
    except (OSError, ValueError) as error:
        raise SystemExit('Cannot validate table bundle: ' + str(error))
