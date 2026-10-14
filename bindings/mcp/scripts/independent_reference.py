"""Independent AST/SciPy Radau integration, without QSPQuest RHS or Jacobian.
Supports the exact Liu/Rajwade archive subset; unsupported constructs fail closed.
"""
import argparse, collections, hashlib, json, math, re, time, zipfile
from pathlib import Path
if not __debug__:
    raise RuntimeError('Numerical qualification requires Python assertions enabled')
import numpy as np
import scipy
from scipy.integrate import solve_ivp
from scipy.sparse import csr_matrix
ANALYTIC_CACHE={}
OPS={'add':'+','subtract':'-','multiply':'*','divide':'/','power':'**','gt':'>','ge':'>=','lt':'<','le':'<=','eq':'==','ne':'!='}
COORD_UNITS={'1','h','(1)/(h)','(1)/(1)'}

def dependencies(e):
    if e['kind']=='symbol':return {e['name']}
    return set().union(*(dependencies(a) for a in e.get('arguments',[])))

def compile_model(d,q,analytic=False):
    assert d['time_unit']=='h' and not d['covariates'] and not q['regimen']['covariate_changes']
    assert not d.get('individual',[]) and not d.get('linear_pk') and not d.get('dose_switches',[]) and not d.get('dose_history',[])
    assert all(s['unit']=='1' and s['initial']['kind']=='quantity' for s in d['states'])
    params={}
    for p in d['parameters']:
        v=q['parameters'].get(p['name'],p['default']);assert v['unit']==p['default']['unit'] and v['unit'] in COORD_UNITS
        params[p['name']]=v['value']
    defs={x['name']:x for x in d['definitions']}; states={s['name']:i for i,s in enumerate(d['states'])}
    switches={s['name']:s for s in d.get('switches',[])}
    symbols=set(params)|set(defs)|set(states)|set(switches)|{'time'}|{'input_'+s for s in states}
    assert all(re.fullmatch(r'[A-Za-z_][A-Za-z_0-9]*',n) for n in symbols)
    def expression(e):
        k=e['kind']
        if k=='literal':return repr(e['value'])
        if k=='quantity':assert e['quantity']['unit']=='1';return repr(e['quantity']['value'])
        if k=='symbol':assert e['name'] in symbols;return e['name']
        if k=='unary':assert e['operator'] in ['negative','positive'];return '('+('-' if e['operator']=='negative' else '+')+expression(e['arguments'][0])+')'
        if k in ['binary','compare']:return '('+expression(e['arguments'][0])+OPS[e['operator']]+expression(e['arguments'][1])+')'
        if k=='call':
            args=[expression(a) for a in e['arguments']]
            if e['name']=='ifelse':return '('+args[1]+' if '+args[0]+' else '+args[2]+')'
            assert e['name'] in ['exp','log','sqrt','abs','min','max','pow','hill'],e['name']
            return e['name']+'('+','.join(args)+')'
        raise ValueError('Unsupported expression '+k)
    order=[];visiting=set();done=set(params)|set(states)|set(switches)|{'time'}|{'input_'+s for s in states}
    def visit(name):
        if name in done:return
        if name in visiting:raise ValueError('cyclic definition')
        visiting.add(name)
        for dep in sorted(dependencies(defs[name]['expression'])):visit(dep)
        visiting.remove(name);done.add(name);order.append(name)
    for name in defs:visit(name)
    expanded={}
    def state_deps(name):
        if name in states:return {states[name]}
        if name in expanded:return expanded[name]
        result=set().union(*(state_deps(n) for n in dependencies(defs[name]['expression']))) if name in defs else set()
        expanded[name]=result;return result
    row=[];col=[]
    for i,s in enumerate(d['states']):
        for j in sorted(set().union(*(state_deps(n) for n in dependencies(s['rhs'])))):row.append(i);col.append(j)
    sparsity=csr_matrix((np.ones(len(row),dtype=bool),(row,col)),shape=(len(states),len(states)))
    head=['    '+s['name']+' = y['+str(i)+']' for i,s in enumerate(d['states'])]
    head+=['    '+name+' = '+expression(defs[name]['expression']) for name in order]
    namespace={**params,**{'input_'+s:0.0 for s in states},**{s:0.0 for s in switches},'exp':math.exp,'log':math.log,'sqrt':math.sqrt,'abs':abs,'min':min,'max':max,'pow':pow,'hill':lambda x,n,k:0.0 if x<=0 else 1.0/(1.0+(k/x)**n),'__builtins__':{}}
    source='def rhs(time,y):\n'+'\n'.join(head)+'\n    return ['+','.join(expression(s['rhs']) for s in d['states'])+']\n'
    source+='def outputs(time,y):\n'+'\n'.join(head)+'\n    return ['+','.join(expression(o['expression']) for o in d['outputs'])+']\n'
    exec(compile(source,'independent_archive_equations','exec'),namespace)
    y0=[]
    for s in d['states']:
        value=q['initial_states'].get(s['name'],s['initial']['quantity']);assert value['unit']=='1';y0.append(value['value'])
    switch_times={}
    for name,s in switches.items():
        assert s.get('comparison')=='lt' and dependencies(s['time'])<=set(params)
        switch_times[name]=float(eval(expression(s['time']),namespace))
    doses=collections.defaultdict(list)
    for dose in q['regimen']['administrations']:
        assert dose['delivery']['kind']=='bolus' and dose['repeat'] is None and dose['lag']['value']==0.0
        assert dose['amount']['unit']=='1' and dose['time']['unit']=='h' and dose['lag']['unit']=='h'
        target=d['states'][states[dose['target']]];assert 'bolus' in target['dosing']['modes'] and target['dosing']['amount_unit']=='1'
        assert dependencies(target['dosing']['scale'])<=set(params)
        amount=dose['amount']['value']*dose['bioavailability']*eval(expression(target['dosing']['scale']),namespace)
        doses[dose['time']['value']].append((states[dose['target']],amount))
    jac=None
    if analytic:
        import sympy as sp
        key=hashlib.sha256(json.dumps(d,sort_keys=True).encode()).hexdigest()
        if key not in ANALYTIC_CACHE:
            syms={name:sp.Symbol(name,real=True) for name in symbols}
            def sym(e):
                k=e['kind']
                if k=='symbol':return syms[e['name']]
                if k=='literal':return sp.Float(repr(e['value']),17)
                if k=='quantity':assert e['quantity']['unit']=='1';return sp.Float(repr(e['quantity']['value']),17)
                a=[sym(x) for x in e.get('arguments',[])]
                if k=='unary':return -a[0] if e['operator']=='negative' else a[0]
                if k=='binary':return {'add':lambda:a[0]+a[1],'subtract':lambda:a[0]-a[1],'multiply':lambda:a[0]*a[1],'divide':lambda:a[0]/a[1],'power':lambda:a[0]**a[1]}[e['operator']]()
                if k=='compare':return {'gt':sp.Gt,'ge':sp.Ge,'lt':sp.Lt,'le':sp.Le,'eq':sp.Eq,'ne':sp.Ne}[e['operator']](*a)
                if k=='call':
                    if e['name']=='ifelse':return sp.Piecewise((a[1],a[0]),(a[2],True))
                    return {'exp':sp.exp,'log':sp.log,'sqrt':sp.sqrt,'abs':sp.Abs,'min':sp.Min,'max':sp.Max,'pow':lambda x,y:x**y}[e['name']](*a)
                raise ValueError(k)
            for name in order:syms[name]=sym(defs[name]['expression'])
            values=[];jr=[];jc=[]
            for i,st in enumerate(d['states']):
                ex=sym(st['rhs'])
                assert not any(str(n).startswith('input_') for n in ex.free_symbols)
                for j in sorted(set().union(*(state_deps(n) for n in dependencies(st['rhs'])))):
                    derivative=sp.diff(ex,syms[d['states'][j]['name']])
                    if derivative!=0:jr.append(i);jc.append(j);values.append(derivative)
            print('generating independent symbolic Jacobian',d['model_id'],len(values),flush=True)
            function=sp.lambdify((syms['time'],[syms[n] for n in states],[syms[n] for n in params],[syms[n] for n in switches]),values,modules='math',cse=True,docstring_limit=0)
            ANALYTIC_CACHE[key]=(function,np.array(jr),np.array(jc))
        function,jr,jc=ANALYTIC_CACHE[key]
        parameters=list(params.values())
        def jac(t,y):
            values=np.asarray(function(t,y,parameters,[namespace[n] for n in switches]),dtype=float)
            if not np.isfinite(values).all():raise ValueError('nonfinite independent symbolic Jacobian')
            return csr_matrix((values,(jr,jc)),shape=(len(states),len(states)))
    return namespace['rhs'],namespace['outputs'],np.array(y0),namespace,sparsity,switch_times,doses,hashlib.sha256(source.encode()).hexdigest(),jac

def integrate(data,rtol,atol,analytic=False):
    d,q,n=data['model'],data['request'],data['result'];g=q['regimen']
    rhs,out,state,namespace,sparsity,switches,doses,sourcehash,jac=compile_model(d,q,analytic)
    begin=g['start']['value'];end=g['end']['value'];times=np.array(n['times']);sides=np.array(n['sides'])
    # Check all explicitly requested observations; the engine supplies implicit event samples too.
    for obs in g['observations']:
        assert np.any((times==obs['time']['value'])&(sides==obs['side']))
    for name,b in switches.items():namespace[name]=float(begin<b)
    bounds=sorted({begin,end,*[t for t in doses if begin<=t<=end],*[t for t in switches.values() if begin<=t<=end],*[c['value'] for c in g['checkpoints'] if begin<=c['value']<=end]})
    refs=np.empty((len(times),len(d['outputs'])));sample_states=np.empty((len(times),len(state)));filled=np.zeros(len(times),dtype=bool)
    nfev=njev=nlu=0;jumps=[];start=time.perf_counter()
    def fill(mask,ys):
        for i,y in zip(np.flatnonzero(mask),ys):sample_states[i]=y;refs[i]=out(times[i],y)
        filled[mask]=True
    for bi,a in enumerate(bounds):
        mask=(times==a)&(sides=='pre')&~filled;fill(mask,[state]*np.count_nonzero(mask))
        before=state.copy();expected=np.zeros(len(state))
        for target,amount in doses.get(a,[]):state[target]+=amount;expected[target]+=amount
        jump_error=state-before-expected
        if doses.get(a):jumps.append({'time':a,'max_scaled_error':float(np.max(np.abs(jump_error)/(1+np.abs(expected)))),'non_dosed_states_exactly_continuous':bool(np.all(jump_error[expected==0]==0))})
        for name,b in switches.items():
            if b==a:namespace[name]=0.0
        mask=(times==a)&(sides=='post')&~filled;fill(mask,[state]*np.count_nonzero(mask))
        if bi+1==len(bounds):continue
        b=bounds[bi+1]
        solution=solve_ivp(rhs,(a,b),state,method='Radau',rtol=rtol,atol=atol,jac=jac,jac_sparsity=sparsity,dense_output=True)
        if not solution.success:raise RuntimeError(solution.message)
        mask=(times>a)&(times<b)&~filled
        if np.any(mask):fill(mask,solution.sol(times[mask]).T)
        state=solution.y[:,-1];nfev+=solution.nfev;njev+=solution.njev;nlu+=solution.nlu
    assert filled.all(),times[~filled];assert np.isfinite(refs).all() and np.isfinite(sample_states).all()
    return refs,sample_states,{'seconds':time.perf_counter()-start,'nfev':nfev,'njev':njev,'nlu':nlu,'jacobian_method':'independent_sympy_symbolic' if analytic else 'scipy_sparse_finite_difference','independent_jacobian_nonzeros':sparsity.nnz,'equation_translation_sha256':sourcehash,'declared_invariants':len(d['invariants']),'finite_outputs_states':True,'bolus_jump_checks':jumps,'switch_boundaries':switches,'min_state':float(sample_states.min())}

def main():
    p=argparse.ArgumentParser();p.add_argument('inputs',type=Path);p.add_argument('output',type=Path);p.add_argument('--case');p.add_argument('--analytic-jac',action='store_true');p.add_argument('--policy',type=Path);args=p.parse_args();args.output.mkdir(parents=True,exist_ok=True)
    rules={r['case']:r for r in json.loads(args.policy.read_text())['cases']} if args.policy else None
    records=[]
    for path in sorted(args.inputs.glob('*.json')):
        if args.case and args.case not in path.name:continue
        if rules is not None and rules[path.stem]['mode']!='independent_radau':continue
        data=json.loads(path.read_text());g=data['request']['regimen'];d=data['model']
        if rules is not None:
            for key,raw in [('input_model_sha256',data['model']),('input_request_sha256',data['request'])]:
                assert hashlib.sha256(json.dumps(raw,sort_keys=True,separators=(',',':'),allow_nan=False).encode()).hexdigest()==rules[path.stem][key]
            assert data['result']['identity']['model_content_hash']==rules[path.stem]['model_content_hash']
            assert data['result']['identity']['run_request_hash']==rules[path.stem]['run_request_hash']
        assert any(s in d['model_id'] for s in ['qspquest_liu_','qspquest_rajwade_'])
        if data.get('result') is None:records.append({'case':path.stem,'native_error':data.get('native_error')});continue
        base_atol=np.array([g['absolute_tolerances'][s['name']]['value'] for s in d['states']]) if g.get('absolute_tolerances') else g['atol']
        record={'case':path.stem,'native_model_hash':data['result']['identity']['model_content_hash'],'native_request_hash':data['result']['identity']['run_request_hash'],'settings':[],'scipy_version':scipy.__version__,'numpy_version':np.__version__}
        refs=[]
        settings=rules[path.stem]['reference_settings'] if rules is not None else [{'rtol':g['rtol']*f,'atol':base_atol*f} for f in [0.1,0.01]]
        for setting in settings:
            level=len(refs)+1;rtol=setting['rtol'];atol=setting['atol']
            try:
                arrays=integrate(data,rtol,atol,args.analytic_jac);refs.append(arrays[0]);np.savez_compressed(args.output/(path.stem+f'--radau-{level}.npz'),outputs=arrays[0],states=arrays[1],times=np.array(data['result']['times']),sides=np.array(data['result']['sides']),names=np.array([o['name'] for o in d['outputs']]),units=np.array([o['unit'] for o in d['outputs']]))
                record['settings'].append({'rtol':rtol,'atol_min':float(np.min(atol)),'atol_max':float(np.max(atol)),**arrays[2]})
                print(path.stem,level,record['settings'][-1]['seconds'],flush=True)
            except Exception as e:
                record['error']=str(e);print(path.stem,'ERROR',str(e),flush=True);break
        (args.output/(path.stem+'.json')).write_text(json.dumps(record,indent=2));records.append(record)
    (args.output/'index.json').write_text(json.dumps({'scope':'Independent archive AST/SciPy Radau/SuperLU numerical references; dimensional qualification remains false','rows':records},indent=2))
if __name__=='__main__':main()
