#!/usr/bin/env python3
"""Scripted MCP workflows and transport checks; this is not an LLM evaluation."""
from __future__ import annotations
import argparse, json, os, socket, subprocess, time, urllib.request, urllib.error
from pathlib import Path

def main():
    parser=argparse.ArgumentParser()
    parser.add_argument("--binary",type=Path)
    parser.add_argument("--url")
    parser.add_argument("--output",type=Path,required=True)
    parser.add_argument("--widget-context",type=Path)
    args=parser.parse_args()
    process=None
    if args.binary:
        with socket.socket() as sock:
            sock.bind(("127.0.0.1",0));port=sock.getsockname()[1]
        args.url=f"http://127.0.0.1:{port}"
        env={**os.environ,"QSPQUEST_MANAGED_ENABLED":"0","QSPQUEST_MCP_PREWARM":"0",
             "QSPQUEST_WIDGET_WORKER_URL":args.url+"/assets/js/engine-worker.mjs"}
        process=subprocess.Popen([str(args.binary),"--http",f"127.0.0.1:{port}"],env=env,
                                 stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
    if not args.url:
        parser.error("Provide --binary or --url")
    base=args.url.removesuffix("/mcp").rstrip("/")
    sequence=0;rows=[];all_checks_passed=False
    def rpc(method,params):
        nonlocal sequence
        sequence+=1
        raw=json.dumps({"jsonrpc":"2.0","id":sequence,"method":method,"params":params}).encode()
        request=urllib.request.Request(base+"/mcp",data=raw,
          headers={"Content-Type":"application/json","Accept":"application/json, text/event-stream",
                   "MCP-Protocol-Version":"2025-11-25"})
        with urllib.request.urlopen(request,timeout=55) as response:
            assert response.headers.get("Mcp-Session-Id") is None
            encoded=response.read()
            # Discovery includes every active input/output schema; tool results have a separate budget.
            # Full-feature discovery carries input and output schemas, separately
            # from the one-MB request limit and the bounded tool-result payload.
            response_limit=2_000_000 if method=="tools/list" else 256_000
            assert len(encoded)<=response_limit,(method,len(encoded))
            value=json.loads(encoded)
            assert "error" not in value,value
            return value["result"]
    def call(name,tool,arguments,expected_error=False):
        start=time.perf_counter();result=rpc("tools/call",{"name":tool,"arguments":arguments})
        value=result.get("structuredContent") or {}
        assert bool(result.get("isError"))==expected_error,(name,value)
        rows.append({"workflow":name,"tool":tool,"expected_error":expected_error,
                     "elapsed_ms":(time.perf_counter()-start)*1000,
                     "library_version":value.get("library_version")})
        return value
    try:
        for _ in range(600):
            if process and process.poll() is not None:
                raise RuntimeError("MCP server exited during startup")
            try:
                with urllib.request.urlopen(base+"/healthz",timeout=.2) as response:
                    assert response.status==200
                    break
            except (OSError,urllib.error.URLError):
                time.sleep(.1)
        else:
            raise RuntimeError("MCP startup timeout")
        init=rpc("initialize",{"protocolVersion":"2025-11-25","capabilities":{},
                              "clientInfo":{"name":"QSPQuest contract runner","version":"0.1.0"}})
        tools=rpc("tools/list",{})
        names={t["name"] for t in tools["tools"]}
        assert {"search_models","fit_population","simulate"}<=names
        assert rpc("resources/list",{})
        assert len(rpc("prompts/list",{})["prompts"])==4
        model_id="qspquest_diffdose_2026_idr_benchmark"
        hits=call("find turnover model","search_models",{"query":"turnover"})
        assert any(m["id"]==model_id for m in hits["models"])
        model=call("inspect source scope and units","get_model",{"model_id":model_id,"detail":"full"})
        assert model["dose_targets"] and "dimensional_qualification" in model["native_execution"]
        scenario=call("copy a versioned scenario","get_scenario_request",{"model_id":model_id,"scenario_id":"higher_exposure"})
        ref={"scenario":{"id":model_id,"scenario_id":"higher_exposure"}}
        original=call("simulate library scenario","simulate",{"model":ref,"outputs":["source_output_0"]})
        changed=call("change elimination rate","simulate",{"model":ref,"parameters":{"source_symbol_5":{"value":.7,"unit":"1"}},"outputs":["source_output_0"]})
        assert original["outputs"][0]["values"]!=changed["outputs"][0]["values"]
        template=call("build two compartments from template","model_template",{"kind":"two_compartment"})
        source=template["canonical_source"]
        bad=source.replace("[L/h]","",1)
        diagnostic=call("reject a missing unit","validate_model",{"source":bad},True)
        assert diagnostic["error"].get("line") is not None,diagnostic
        call("recover by restoring the unit","validate_model",{"source":source})
        assets=Path(__file__).resolve().parents[1]/"assets"
        scalar=json.loads((assets/"fits/scalar.json").read_text())["arguments"]
        iv=scalar["model"]
        request=scalar["request"];request["regimen"]["end"]={"value":336,"unit":"h"}
        request["regimen"].pop("observations",None);request["regimen"]["samples"]=[{"value":i*336/100,"unit":"h"} for i in range(101)]
        def doses(amount,interval,count):
            return [{"target":"central","amount":{"value":amount,"unit":"mg"},
                     "at":{"value":0,"unit":"h"},"count":count,**({"every":{"value":interval,"unit":"h"}} if count>1 else {})}]
        compared=call("compare equal 14-day amounts QW versus Q2W","compare_regimens",
                      {"model":iv,"request":request,"output":"cp",
                       "regimens":[{"name":"QW","doses":doses(100,168,2)},
                                   {"name":"Q2W","doses":doses(200,336,1)}]})
        assert len(compared["regimens"])==2
        scan=call("scan clearance with units","scan_parameters",{"model":iv,"request":scalar["request"],
                 "parameter":"cl","values":[{"value":v,"unit":"L/h"} for v in [.2,.4,.6]],"output":"cp"})
        assert len(scan["results"])==3
        individual=json.loads((assets/"fits/individual-additive.json").read_text())
        fitted=call("fit two parameters to synthetic observations","fit_individual",individual["arguments"])
        assert fitted["fit"]["status"]=="converged"
        population=json.loads((assets/"fits/population.json").read_text())
        fitted=call("map NONMEM rows and fit small population","fit_population",population["arguments"])
        assert fitted["population"]["status"]=="converged"
        assert len(rows)==12
        if args.widget_context:
            oral=(assets/"templates/oral.qsp").read_text()
            widget=rpc("tools/call",{"name":"simulate","arguments":{"model":{"source":oral},
                "doses":[{"target":"depot","amount":{"value":100,"unit":"mg"},"at":{"value":0,"unit":"h"}}],
                "end":{"value":12,"unit":"h"},"outputs":["cp"]}})
            assert not widget.get("isError"),widget
            value=widget["structuredContent"]
            if not value["widget_context"].get("worker_url"):
                value["widget_context"]["worker_url"]=base+"/assets/js/engine-worker.mjs"
            args.widget_context.write_text(json.dumps(value))
        # A deep left-associated expression must not be parsed on the HTTP thread.
        # It is inspected inside Wasmtime, and rejection must leave the process usable.
        prefix="model deep_candidate\ntime_unit = h\nparameters\n rate = 1 [1/h] fixed\nstates\n x = 1 [1]\ndynamics\n d/dt(x) = "
        suffix="\noutputs\n x_out = x [1]\nend\n"
        control=rpc("tools/call",{"name":"validate_model","arguments":{"source":prefix+"rate * x"+suffix}})
        assert not control.get("isError"),control
        bad_source=prefix+" + ".join(["rate * x"]*20000)+suffix
        rejected=rpc("tools/call",{"name":"simulate","arguments":{"model":{"source":bad_source},"end":{"value":1,"unit":"h"}}})
        assert rejected.get("isError"),rejected
        containment={"valid_control":True,"deep_source_rejected":True,"error":rejected.get("structuredContent"),"source_bytes":len(bad_source)}
        assert not rpc("tools/call",{"name":"search_models","arguments":{"query":"turnover"}}).get("isError")
        oversized=urllib.request.Request(base+"/mcp",data=b"x"*1_000_001,
            headers={"Content-Type":"application/json","Accept":"application/json, text/event-stream"})
        try:
            urllib.request.urlopen(oversized,timeout=5)
            raise AssertionError("Oversized HTTP payload accepted")
        except urllib.error.HTTPError as error:
            assert error.code==413,error.code
        request=rpc("resources/read",{"uri":"qspquest://runs/nonexistent"})
        raise AssertionError("Hosted run resource unexpectedly available")
    except AssertionError as error:
        # Hosted resources/read uses a JSON-RPC invalid-params error, detected by rpc.
        if str(error).find("Run resources exist only in local stdio mode")<0:
            raise
        all_checks_passed=True
    finally:
        args.output.write_text(json.dumps({"kind":"scripted_contract_workflows","not_an_llm_evaluation":True,
            "transport":"stateless streamable HTTP","all_checks_passed":all_checks_passed,"workflows":rows,"all_workflows_passed":len(rows)==12,"source_containment":locals().get("containment")},indent=2))
        if process:
            process.terminate()
            try:process.wait(timeout=5)
            except subprocess.TimeoutExpired:process.kill();process.wait()
if __name__=="__main__":main()
