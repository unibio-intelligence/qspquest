"""Exercise verified sessions against a mock platform, never a production account."""
import hashlib,math,json,os,socket,subprocess,threading,time,urllib.request,urllib.error
from http.server import BaseHTTPRequestHandler,ThreadingHTTPServer
from pathlib import Path
import argparse
parser=argparse.ArgumentParser();parser.add_argument('--binary',type=Path,required=True);parser.add_argument('--output',type=Path,required=True);args=parser.parse_args()
seen=[];previews={};submissions={}
def digest(body):
    return hashlib.sha256(json.dumps(body,sort_keys=True,separators=(',',':')).encode()).hexdigest()
def price():
    return {'credit_cost':42,'billing_unit':'credits','pricing_model':'fixed_fixture','pricing_note':'Synthetic fixed operation'}
def job(capability,state='queued'):
    suffix='fit' if capability=='pkpd.fit_data' else 'simulation'
    return {'id':'job-'+suffix,'run_id':'run-'+suffix,'destination_project_id':'project-fixture','state':state,'pricing':price(),'staged_outputs':[]}
def success(result):
    assert not result.get('isError',False),result
    return result['structuredContent']
class Platform(BaseHTTPRequestHandler):
    def log_message(self,*_):pass
    def reply(self,value,status=200):
        raw=json.dumps(value).encode();self.send_response(status);self.send_header('Content-Type','application/json');self.send_header('Content-Length',str(len(raw)));self.end_headers();self.wfile.write(raw)
    def do_GET(self):
        auth=self.headers.get('Authorization');tier={'Bearer free-fixture':'free','Bearer pro-fixture':'pro'}.get(auth)
        if self.path=='/session':
            return self.reply({'user_id':'principal-fixture','tenant_id':'tenant-fixture','tier':tier},200) if tier else self.reply({'error':'unauthorized'},401)
        assert tier=='pro',self.path
        seen.append(self.path)
        if self.path=='/pkpd/models':self.reply({'data':{'models':[{'id':'private-fixture','name':'Private fixture','access':{'tier_locked':False}}]}})
        elif self.path=='/pkpd/models/private-fixture':self.reply({'data':{'id':'private-fixture','operation_readiness':{'population_fit':False},'scope':'mock catalog metadata'}})
        elif self.path=='/jobs/job-fit':self.reply({'data':job('pkpd.fit_data','succeeded')})
        elif self.path=='/artifacts/artifact-fixture':self.reply({'data':{'id':'artifact-fixture','organization_id':'tenant-fixture','project_id':'project-fixture','run_id':'run-fit','job_id':'job-fit','kind':'population_fit','name':'Synthetic fit metadata','media_type':'application/json','size_bytes':2,'checksum_hex':hashlib.sha256(b'{}').hexdigest(),'download_href':'/artifacts/artifact-fixture/download'}})
        else:self.reply({'error':'not found'},404)
    def do_POST(self):
        assert self.headers.get('Authorization')=='Bearer pro-fixture'
        body=json.loads(self.rfile.read(int(self.headers['Content-Length'])));seen.append(self.path)
        assert body['capability_id'] in {'pkpd.fit_data','pkpd.simulate_scenario'}
        if self.path=='/runs/preview':
            assert 'expected_preview_digest' not in body
            reviewed=digest(body);previews[reviewed]=body
            self.reply({'data':{'capability_id':body['capability_id'],'capability_version':'1','valid':True,'preview_digest':reviewed,'approval_required':False,'pricing':price(),'workbench_href':'/workbench'}})
        elif self.path=='/runs':
            reviewed=body.get('expected_preview_digest');original={k:v for k,v in body.items() if k!='expected_preview_digest'}
            if reviewed not in previews or previews[reviewed]!=original:
                return self.reply({'error':'preview mismatch'},409)
            key=self.headers.get('Idempotency-Key');assert key in {'retry-simulation','retry-fit'}
            identity=digest(body)
            if key in submissions:
                if submissions[key][0]!=identity:return self.reply({'error':'retry input changed'},409)
                return self.reply({'data':submissions[key][1]})
            descriptor=job(body['capability_id'])
            value={'run':{'id':descriptor['run_id'],'destination_project_id':'project-fixture','capability_id':body['capability_id'],'capability_version':'1','state':'ready'},'jobs':[descriptor],'pricing':price(),'status_href':'/runs/'+descriptor['run_id'],'workbench_href':'/workbench/'+descriptor['run_id']}
            submissions[key]=(identity,value);self.reply({'data':value})
        else:self.reply({'error':'not found'},404)
platform=ThreadingHTTPServer(('127.0.0.1',0),Platform)
threading.Thread(target=platform.serve_forever,daemon=True).start()
with socket.socket() as sock:sock.bind(('127.0.0.1',0));port=sock.getsockname()[1]
base=f'http://127.0.0.1:{port}'
api=f'http://127.0.0.1:{platform.server_port}'
env={**os.environ,'QSPQUEST_LIBRARY_DIR':str(Path(__file__).resolve().parents[1]/'assets/library'),'QSPQUEST_MCP_PREWARM':'0','QSPQUEST_MANAGED_ENABLED':'1','QSPQUEST_PLATFORM_API_URL':api,'QSPQUEST_PLATFORM_SESSION_URL':api+'/session'}
server_log=args.output.with_suffix('.server.log')
server_output=server_log.open('w')
process=subprocess.Popen([str(args.binary.resolve()),'--http',f'127.0.0.1:{port}'],env=env,stdout=server_output,stderr=subprocess.STDOUT)
def call(token,name,arguments):
    raw=json.dumps({'jsonrpc':'2.0','id':1,'method':'tools/call','params':{'name':name,'arguments':arguments}}).encode()
    req=urllib.request.Request(base+'/mcp',data=raw,headers={'Authorization':'Bearer '+token,'Content-Type':'application/json','Accept':'application/json, text/event-stream','MCP-Protocol-Version':'2025-11-25','X-User-Tier':'admin'})
    with urllib.request.urlopen(req,timeout=20) as response:
        assert response.headers.get('Mcp-Session-Id') is None
        payload=json.load(response);assert 'error' not in payload,payload
        return payload['result']
try:
    for _ in range(300):
        if process.poll() is not None:raise RuntimeError(f'Managed server exited; inspect {server_log}')
        try:
            urllib.request.urlopen(base+'/healthz',timeout=.1).close();break
        except OSError:time.sleep(.1)
    else:raise RuntimeError(f'Managed startup timeout; inspect {server_log}')
    request={'platform_request':{'capability_id':'pkpd.fit_data','destination_project_id':'project-fixture','inputs':[],'parameters':{'synthetic':True}},'idempotency_key':'retry-fit'}
    public=call('free-fixture','search_models',{'query':'turnover'})
    assert public['structuredContent']['models']
    assert 'platform_models' not in public['structuredContent']
    assert call('free-fixture','get_model',{'model_id':'private-fixture'})['isError']
    assert call('free-fixture','submit_population_fit',request)['isError']
    assert not seen,'Free request reached a private catalog or job route'
    private=call('pro-fixture','search_models',{'query':'private'})
    assert private['structuredContent']['platform_models'][0]['id']=='private-fixture'
    assert call('pro-fixture','get_model',{'model_id':'private-fixture'})['structuredContent']['platform_model']['data']['id']=='private-fixture'
    simulation={**request,'platform_request':{**request['platform_request'],'capability_id':'pkpd.simulate_scenario'},'idempotency_key':'retry-simulation'}
    assert call('free-fixture','submit_simulation',simulation)['isError']
    before=len(seen)
    for invalid in (simulation,{**simulation,'platform_request':{**simulation['platform_request'],'expected_preview_digest':'fixture-preview'}}):
        rejected=call('pro-fixture','submit_simulation',invalid)
        assert rejected['isError'] and rejected['structuredContent']['error']['code']=='preview_required',rejected
    assert len(seen)==before,'Unreviewed input reached a priced submission'
    preview=success(call('pro-fixture','preview_job',{'platform_request':simulation['platform_request']}))['managed_preview']
    assert preview['valid'] and not preview['approval_required'] and preview['pricing']==price()
    simulation['platform_request']['expected_preview_digest']=preview['preview_digest']
    changed={**simulation,'platform_request':{**simulation['platform_request'],'parameters':{'synthetic':'changed'}}}
    assert call('pro-fixture','submit_simulation',changed)['isError'],'Changed input reused a reviewed preview'
    before=len(seen)
    timings=[]
    accepted=None
    for _ in range(20):
        start=time.perf_counter()
        result=call('pro-fixture','submit_simulation',simulation)
        timings.append((time.perf_counter()-start)*1000)
        submission=success(result)['managed_submission']
        assert submission['jobs'][0]['state']=='queued' and submission['pricing']==preview['pricing']
        if accepted is None:accepted=submission
        assert submission==accepted,'Same retry key created a different submission'
    assert seen[before:]==['/runs']*20,'Submission performed an implicit price preview'
    assert len(submissions)==1,'Simulation retries created extra Runs'
    fit_preview=success(call('pro-fixture','preview_job',{'platform_request':request['platform_request']}))['managed_preview']
    assert fit_preview['pricing']==price()
    request['platform_request']['expected_preview_digest']=fit_preview['preview_digest']
    submitted=success(call('pro-fixture','submit_population_fit',request))['managed_submission']
    assert submitted['jobs'][0]['id']=='job-fit' and submitted['pricing']==fit_preview['pricing']
    assert len(submissions)==2
    assert success(call('pro-fixture','wait_for_job',{'job_id':'job-fit','seconds':1}))['managed_job']['state']=='succeeded'
    artifact=success(call('pro-fixture','get_artifact',{'artifact_id':'artifact-fixture'}))
    assert artifact['managed_artifact']['id']=='artifact-fixture'
    assert artifact['artifact_retrieval']['state']=='unavailable' and 'full_result_resource' not in artifact
    try:call('unverified-fixture','search_models',{'query':''});raise AssertionError('Unverified bearer accepted')
    except urllib.error.HTTPError as e:assert e.code==401
    args.output.write_text(json.dumps({'kind':'mock_platform_http_contract','simulation_preview_run_contract_passed':True,'mock_job_admission_p95_ms':sorted(timings)[math.ceil(.95*len(timings))-1],'mock_job_admission_samples_ms':timings,'live_managed_qualification':False,'free_private_calls':0,'caller_tier_hint_ignored':True,'unverified_session_rejected':True,'pro_private_catalog_visible':True,'governed_preview_run_job_artifact_passed':True,'missing_or_malformed_preview_refused':True,'changed_input_preview_refused':True,'same_retry_submission_preserved':True,'created_mock_runs':len(submissions),'legacy_artifact_metadata_not_bytes':True,'routes':seen},indent=2))
finally:
    process.terminate();process.wait(timeout=5);server_output.close();platform.shutdown();platform.server_close()
