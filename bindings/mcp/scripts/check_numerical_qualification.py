"""Check frozen output bounds against independently integrated reference arrays."""
import argparse,copy,hashlib,json,math
from pathlib import Path
if not __debug__:
    raise RuntimeError('Numerical qualification requires Python assertions enabled')
import numpy as np
from independent_reference import compile_model,dependencies

def input_hash(value):
    return hashlib.sha256(json.dumps(value,sort_keys=True,separators=(',',':'),allow_nan=False).encode()).hexdigest()

def matrix(result):
    return np.array([o['values'] for o in result['outputs']],dtype=float).T

def validate_result(result,model,request,names,units):
    """Bind every result row to the frozen request before comparing targets."""
    samples=request['regimen']['observations']
    times=result.get('times');sides=result.get('sides');outputs=result.get('outputs')
    if not samples or not names or len(names)!=len(set(names)):
        raise ValueError('Nonempty observations and unique output names are required')
    if (not isinstance(times,list) or len(times)!=len(samples)
        or not isinstance(sides,list) or len(sides)!=len(samples)
        or result.get('time_unit')!=model['time_unit']):
        raise ValueError('Result observation count or time unit differs')
    for sample,time,side in zip(samples,times,sides):
        expected=sample['time']
        if (expected['unit']!=model['time_unit'] or type(time) not in (int,float)
            or type(expected['value']) not in (int,float) or not math.isfinite(time)
            or not math.isfinite(expected['value']) or time!=expected['value']
            or side not in ('pre','post') or side!=sample.get('side','post')):
            raise ValueError('Result time grid or event sides differ from request')
    if (not isinstance(outputs,list) or [o['name'] for o in outputs]!=names
        or [o['unit'] for o in outputs]!=units):
        raise ValueError('Result output contract differs')
    for output in outputs:
        values=output['values']
        if (not isinstance(values,list) or len(values)!=len(samples)
            or any(type(v) not in (int,float) or not math.isfinite(v) for v in values)):
            raise ValueError('Result values must be complete finite numeric vectors')

def score(actual,expected,absolute,relative,reference_only=False):
    assert actual.shape==expected.shape and np.isfinite(actual).all() and np.isfinite(expected).all()
    budget=absolute+relative*(np.abs(expected) if reference_only else np.maximum(np.abs(actual),np.abs(expected)))
    return np.max(np.abs(actual-expected)/budget,axis=0)

def constants(model,request):
    """Prove constant states/readouts from dependencies and zero initial derivatives."""
    rhs,out,initial,ns,_,_,doses,_,_=compile_model(model,request)
    definitions={d['name']:d['expression'] for d in model['definitions']}
    parameters={p['name'] for p in model['parameters']}
    def expanded(e):
        result=set()
        for name in dependencies(e):
            result |= expanded(definitions[name]) if name in definitions else {name}
        return result
    dosed={model['states'][i]['name'] for entries in doses.values() for i,_ in entries}
    fixed=set();initial_rhs=rhs(request['regimen']['start']['value'],initial)
    for _ in model['states']:
        added={s['name'] for i,s in enumerate(model['states']) if s['name'] not in dosed and initial_rhs[i]==0 and expanded(s['rhs'])<=parameters|fixed}
        if added<=fixed:break
        fixed |= added
    values=out(request['regimen']['start']['value'],initial)
    return {o['name']:values[i] for i,o in enumerate(model['outputs']) if expanded(o['expression'])<=parameters|fixed},fixed

def event_checks(data,result,absolute,relative):
    model,request=data['model'],data['request']
    state_columns={o['expression'].get('name'):i for i,o in enumerate(model['outputs']) if o['expression']['kind']=='symbol'}
    indices=[state_columns[s['name']] for s in model['states']]
    states=matrix(result)[:,indices];times=np.array(result['times']);sides=np.array(result['sides'])
    _,_,_,_,_,_,doses,_,_=compile_model(model,request)
    rows=[]
    for time,entries in doses.items():
        pre=np.flatnonzero((times==time)&(sides=='pre'));post=np.flatnonzero((times==time)&(sides=='post'))
        assert len(pre)==len(post)==1,'Missing/duplicate dose-side sample'
        before,after=states[pre[0]],states[post[0]];expected=np.zeros(len(indices))
        for i,amount in entries:expected[i]+=amount
        error=np.abs(after-before-expected)
        budget=absolute[indices]+relative[indices]*np.maximum.reduce([np.abs(before),np.abs(after),np.abs(expected)])
        rows.append({'time':time,'max_budget_fraction':float(np.max(error/budget)),
                     'non_dosed_states_exactly_continuous':bool(np.array_equal(before[expected==0],after[expected==0]))})
    return rows

def check_case(data,rule,reference_dir,case,event_data=None):
    native,wasm=data['result'],data['wasm_result'];model=data['model']
    assert input_hash(model)==rule['input_model_sha256'] and input_hash(data['request'])==rule['input_request_sha256'],'Input bytes changed from reviewed policy'
    assert all(native['identity'][k]==wasm['identity'][k] for k in ('model_content_hash','bound_value_hash','run_request_hash'))
    assert native['identity']['model_content_hash']==rule['model_content_hash'] and native['identity']['run_request_hash']==rule['run_request_hash'],'Stale numerical policy'
    names=[o['name'] for o in rule['outputs']];units=[o['unit'] for o in rule['outputs']]
    for result in (native,wasm):validate_result(result,model,data['request'],names,units)
    assert all([o['name'] for o in r['outputs']]==names and [o['unit'] for o in r['outputs']]==units for r in (native,wasm)),'Output contract mismatch'
    assert native['times']==wasm['times'] and native['sides']==wasm['sides'],'Event grid mismatch'
    absolute=np.array([o['absolute'] for o in rule['outputs']]);relative=np.array([o['relative'] for o in rule['outputs']]);a,b=matrix(native),matrix(wasm)
    cross=score(a,b,absolute,relative)
    row={'case':case,'mode':rule['mode'],'cross_target_max_budget_fraction':float(cross.max()),
         'strict_absolute_1e9_pass':bool(np.max(np.abs(a-b))<=1e-9),'qualified':bool(cross.max()<=1),
         'per_output_cross_target_max_budget_fraction':cross.tolist()}
    if rule['mode']=='legacy_absolute_regression':return row
    refs=[np.load(reference_dir/(case+f'--radau-{i}.npz')) for i in (1,2)]
    stats=json.loads((reference_dir/(case+'.json')).read_text());assert not stats.get('error') and len(stats['settings'])==2
    assert stats['native_model_hash']==rule['model_content_hash'] and stats['native_request_hash'] in (rule['run_request_hash'],rule['reference_base_request_hash'])
    assert model['invariants']==[],'Declared invariant unsupported by the independent adapter'
    assert all(np.array_equal(r['times'],native['times']) and np.array_equal(r['sides'],native['sides']) and r['names'].tolist()==names and r['units'].tolist()==units for r in refs)
    reference=refs[1]['outputs'];refine=score(refs[0]['outputs'],reference,absolute,relative,True);na=score(a,reference,absolute,relative,True);wa=score(b,reference,absolute,relative,True)
    fixed,fixed_states=constants(model,data['request']);constant_error=max((float(np.max(np.abs(x[:,names.index(k)]-v))) for x in (a,b,reference) for k,v in fixed.items()),default=0.)
    assert event_data is not None,'Dose-side diagnostics required'
    assert event_data['model']==model
    original=json.loads(json.dumps(data['request']));probe=json.loads(json.dumps(event_data['request']))
    observations=original['regimen'].pop('observations');probe_observations=probe['regimen'].pop('observations')
    assert original==probe,'Event probe changed more than sampling'
    allowed=list(observations)
    for dose in data['request']['regimen']['administrations']:
        allowed += [{'time':dose['time'],'side':side} for side in ('pre','post')]
    assert all(o in probe_observations for o in observations) and all(o in allowed for o in probe_observations)
    en,ew=event_data['result'],event_data['wasm_result']
    for result in (en,ew):validate_result(result,model,event_data['request'],names,units)
    assert en['times']==ew['times'] and en['sides']==ew['sides']
    assert all([o['name'] for o in r['outputs']]==names and [o['unit'] for o in r['outputs']]==units for r in (en,ew))
    assert all(en['identity'][k]==ew['identity'][k] for k in ('model_content_hash','bound_value_hash','run_request_hash'))
    assert en['identity']['model_content_hash']==rule['model_content_hash']
    assert score(matrix(en),matrix(ew),absolute,relative).max()<=1
    events={target:event_checks(event_data,result,absolute,relative) for target,result in [('native',en),('wasm',ew)]}
    row.update(reference_refinement_max_budget_fraction=float(refine.max()),native_reference_max_budget_fraction=float(na.max()),wasm_reference_max_budget_fraction=float(wa.max()),
               per_output_native_reference_max_budget_fraction=na.tolist(),per_output_wasm_reference_max_budget_fraction=wa.tolist(),
               important_native_max_budget_fraction=float(max(na[i] for i,o in enumerate(rule['outputs']) if o.get('important'))),
               important_wasm_max_budget_fraction=float(max(wa[i] for i,o in enumerate(rule['outputs']) if o.get('important'))),
               constant_outputs_checked=len(fixed),constant_states_proved=len(fixed_states),constant_max_absolute_error=constant_error,
               declared_physical_invariants=0,dimensional_qualification=False,bolus_checks=events)
    row['qualified'] &= bool(refine.max()<=.01 and max(na.max(),wa.max())<=.25 and constant_error<=1e-9 and
                             all(e['non_dosed_states_exactly_continuous'] and e['max_budget_fraction']<=.25 for checks in events.values() for e in checks))
    return row

def negative_controls(data,rule,references,case,event_data):
    ref=np.load(references/(case+'--radau-2.npz'))['outputs']
    column=next(i for i,o in enumerate(rule['outputs']) if o.get('important') and np.ptp(ref[:,i])>0)
    wrong=copy.deepcopy(data)
    for target in ('result','wasm_result'):
        wrong[target]['outputs'][column]['values']=[v*1.01 for v in wrong[target]['outputs'][column]['values']]
    row=check_case(wrong,rule,references,case,event_data)
    assert row['strict_absolute_1e9_pass'] and not row['qualified'],'Shared wrong result escaped independent check'
    changed=copy.deepcopy(data);changed['request']['description']='stale-input-control'
    def rejected(d,e):
        try:
            return not check_case(d,rule,references,case,e)['qualified']
        except (AssertionError,KeyError,ValueError):
            return True
    assert rejected(changed,event_data),'Stale input escaped reviewed hash'
    sides=copy.deepcopy(event_data)
    for target in ('result','wasm_result'):
        sides[target]['sides']=['post' if s=='pre' else s for s in sides[target]['sides']]
    assert rejected(data,sides),'Missing predose sample escaped event check'
    return {'case':case,'shared_wrong_results_rejected':True,'stale_input_rejected':True,'dose_side_mutation_rejected':True}

def main():
    p=argparse.ArgumentParser();p.add_argument('inputs',type=Path);p.add_argument('references',type=Path);p.add_argument('policy',type=Path);p.add_argument('report',type=Path);p.add_argument('--event-inputs',type=Path,required=True);args=p.parse_args()
    policy=json.loads(args.policy.read_text());rows=[]
    cases=[r['case'] for r in policy['cases']]
    assert len(cases)==len(set(cases))==policy['expected_scenarios']
    assert set(cases)=={p.stem for p in args.inputs.glob('*.json')},'Scenario coverage mismatch'
    assert sum(r['mode']=='independent_radau' for r in policy['cases'])==policy['expected_independent_scenarios']
    controls=None
    for rule in policy['cases']:
        case=rule['case'];data=json.loads((args.inputs/(case+'.json')).read_text())
        try:
            event_data=json.loads((args.event_inputs/(case+'.json')).read_text()) if rule['mode']=='independent_radau' else None
            row=check_case(data,rule,args.references,case,event_data)
            if controls is None and rule['mode']=='independent_radau' and row['qualified'] and data['request']['regimen']['administrations']:
                controls=negative_controls(data,rule,args.references,case,event_data)
        except Exception as error:
            row={'case':case,'qualified':False,'error':str(error) or type(error).__name__}
        rows.append(row)
    report={'schema':'qspquest.numerical-qualification/v1','policy_schema':policy['schema'],'passed':sum(r['qualified'] for r in rows),'total':len(rows),'negative_controls':controls,'rows':rows}
    args.report.write_text(json.dumps(report,indent=2));print(json.dumps({k:v for k,v in report.items() if k!='rows'}))
    for row in rows:
        if not row['qualified']:print(row['case'],row.get('error') or 'Numerical budget exceeded')
    if report['passed']!=report['total'] or controls is None:raise SystemExit(1)

if __name__=='__main__':main()
