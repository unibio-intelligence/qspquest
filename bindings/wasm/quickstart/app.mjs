import { createEngineClient } from '../js/index.mjs';
const $ = id => document.getElementById(id), client = createEngineClient();
const inputs = ['dose', 'clearance', 'volume'];
let csv, downloadUrl, running = false, current, lastResult, baseline, library=[];
const byteLength = text => new TextEncoder().encode(text).length;
function state(message,error=false) { $('status').textContent=message;$('status').dataset.error=String(error); }
function clearResult() {
  csv=undefined;current=undefined;$('download').disabled=true;
  for (const id of ['plot','summary','table-panel','engine-panel','comparison','output-control']) $(id).setAttribute('hidden','');
}
function syncControls() {
  const guided=$('model-picker').value==='synthetic';
  const editableParameter=$('model-picker').value.startsWith('library-')&&!$('library-controls').hidden;
  for (const id of ['run','restore','model-picker']) $(id).disabled=running;
  for (const id of ['model-file','request-file','source','request']) $(id).disabled=running||guided;
  for (const id of inputs) { $(id).disabled=running||!guided;$(id).required=guided; }
  for (const id of ['parameter','parameter-value']) $(id).disabled=running||!editableParameter;
  $('parameter-value').required=editableParameter;
  $('cancel').disabled=!running;
}
function busy(value) { running=value;syncControls(); }
function refreshParameters() {
  const selected=$('parameter').value;
  $('parameter').replaceChildren();
  if ($('model-picker').value.startsWith('library-')) {
    try {
      const source=JSON.parse($('source').value),request=JSON.parse($('request').value);
      for (const p of source.parameters) {
        if (p.fixed||typeof request.parameters?.[p.name]?.value!=='number') continue;
        const option=document.createElement('option');option.value=p.name;option.textContent=p.description||p.name;$('parameter').append(option);
      }
      if (Array.from($('parameter').options).some(p=>p.value===selected)) $('parameter').value=selected;
    } catch { /* Incomplete JSON remains editable; execution reports the diagnostic. */ }
  }
  $('library-controls').hidden=!$('parameter').options.length;
  if (!$('library-controls').hidden) selectParameter();
  syncControls();
}
async function load() {
  const responses=await Promise.all([fetch('../../../conformance/models/synthetic-one-compartment.qsp'),fetch('../../../docs/examples/first-run.json'),fetch('./library.json')]);
  if (responses.some(r=>!r.ok)) throw new Error('An example asset is missing. Serve the complete browser bundle over HTTP.');
  baseline={source:await responses[0].text(),request:await responses[1].json()};
  library=await responses[2].json();
  if (!Array.isArray(library)) throw new Error('The library menu has an invalid format.');
  library.forEach((model,i)=>{
    const option=document.createElement('option');option.value=`library-${i}`;option.textContent=`${model.name} · ${model.version} · ${model.scenario}`;$('model-picker').append(option);
  });
  $('source').value=baseline.source;$('request').value=JSON.stringify(baseline.request,null,2);
}
busy(true);const ready=load().then(()=>state('Ready. Select Run simulation to start.')).catch(error=>{state(error.message,true);throw error;}).finally(()=>busy(false));
ready.catch(()=>{});
function selectModel() {
  clearResult();lastResult=undefined;
  const choice=$('model-picker').value,guided=choice==='synthetic';
  $('guided').hidden=!guided;$('custom-inputs').hidden=guided;
  $('editors').open=choice==='custom';$('library-controls').hidden=!choice.startsWith('library-');
  $('guided-explanation').hidden=!guided;$('guided-note').hidden=!guided;
  $('model-description').textContent=guided?'A synthetic teaching model with dose, clearance and volume controls.':'Provide a QSPQuest model and compatible versioned run request. The engine uses the declared model and request units.';
  if (choice.startsWith('library-')) {
    const model=library[Number(choice.slice(8))];
    $('source').value=model.source;$('request').value=JSON.stringify(model.request,null,2);
    $('model-description').textContent=`${model.name} · version ${model.version} · ${model.scenario}. ${model.description} License: ${model.license}.`;
  }
  refreshParameters();
  state('Model selected. Inspect the inputs, then run simulation.');
}
function selectParameter() {
  try {
    const request=JSON.parse($('request').value),value=request.parameters[$('parameter').value];
    $('parameter-value').value=value.value;$('parameter-label').textContent=`Value (${value.unit})`;
  }catch(error){state(`Cannot edit this parameter: ${error.message}`,true);}
}
function outputName(result,index) {return current?.outputLabels?.[result.names[index]]||result.names[index];}
function chart(result,index,previous) {
  const svg=$('plot'),times=Array.from(result.times),values=Array.from(result.columns[index]);
  svg.querySelectorAll('g').forEach(node=>node.remove());
  const group=document.createElementNS(svg.namespaceURI,'g');svg.append(group);
  const add=(tag,attrs,text)=>{const node=document.createElementNS(svg.namespaceURI,tag);for(const [k,v] of Object.entries(attrs))node.setAttribute(k,v);if(text!==undefined)node.textContent=text;group.append(node);};
  const all=[...values,...(previous?.values||[])],lo=Math.min(0,...all),hi=Math.max(...all),span=hi-lo||1;
  const bottom=lo-span*.05,top=hi+span*.1,start=times[0],end=times.at(-1),duration=end-start||1;
  const x=t=>62+(t-start)/duration*546,y=v=>276-(v-bottom)/(top-bottom)*244;
  for(let n=0;n<=4;n++){const v=bottom+(top-bottom)*n/4;add('line',{x1:62,x2:608,y1:y(v),y2:y(v),class:'grid'});add('text',{x:54,y:y(v)+4,'text-anchor':'end'},Number(v.toPrecision(3)).toString());}
  for(let n=0;n<=4;n++){const t=start+duration*n/4;add('text',{x:x(t),y:299,'text-anchor':'middle'},Number(t.toPrecision(4)).toString());}
  add('line',{x1:62,x2:608,y1:276,y2:276,class:'axis'});
  add('text',{x:335,y:327,'text-anchor':'middle'},`Time (${result.time_unit})`);
  add('text',{x:18,y:150,transform:'rotate(-90 18 150)','text-anchor':'middle'},`${outputName(result,index)} (${result.units[index]})`);
  if(previous)add('path',{d:previous.times.map((t,i)=>(i?'L':'M')+x(t)+','+y(previous.values[i])).join(' '),class:'previous-curve'});
  add('path',{d:times.map((t,i)=>(i?'L':'M')+x(t)+','+y(values[i])).join(' '),class:'curve'});
  svg.removeAttribute('hidden');
}
function render(index) {
  const {result,key}=current,times=Array.from(result.times),values=Array.from(result.columns[index]);
  const priorIndex=lastResult?.result.names.indexOf(result.names[index])??-1;
  const comparable=lastResult&&lastResult.key!==key&&lastResult.modelKey===current.modelKey&&lastResult.result.model_id===result.model_id&&priorIndex>=0&&lastResult.result.time_unit===result.time_unit&&lastResult.result.units[priorIndex]===result.units[index]&&lastResult.result.times[0]===times[0]&&lastResult.result.times.at(-1)===times.at(-1);
  const previous=comparable?{times:Array.from(lastResult.result.times),values:Array.from(lastResult.result.columns[priorIndex])}:undefined;
  chart(result,index,previous);
  $('comparison').hidden=!previous;$('comparison').textContent=previous?`Solid green: current run. Dashed blue: previous run (${lastResult.label}).`:'';
  $('rows').replaceChildren();
  const selected=times.length<=100?times.map((_,i)=>i):Array.from({length:100},(_,i)=>Math.round(i*(times.length-1)/99));
  for(const i of selected){const row=document.createElement('tr');for(const value of [String(times[i]),values[i].toPrecision(9),result.sides[i]===1?'post':'pre']){const cell=document.createElement('td');cell.textContent=value;row.append(cell);}$('rows').append(row);}
  $('time-heading').textContent=`Time (${result.time_unit})`;$('value-heading').textContent=`${outputName(result,index)} (${result.units[index]})`;
  $('summary').textContent=`${times.length} observations · Start: ${values[0].toPrecision(5)} ${result.units[index]} · End: ${values.at(-1).toPrecision(5)} ${result.units[index]}. ${times.length>100?'Table shows 100 rows; CSV contains every observation.':''}`;
  $('identity').textContent=JSON.stringify({engine:result.identity,model_id:result.model_id,description:result.description,time_unit:result.time_unit,outputs:result.names,units:result.units},null,2);
  for(const id of ['summary','table-panel','engine-panel','output-control'])$(id).hidden=false;
  // Long format keeps different output units and event sides explicit.
  const quote=value=>`"${String(value).replaceAll('"','""')}"`;
  csv='time,time_unit,observation_side,output,unit,value\n'+times.flatMap((t,i)=>result.names.map((name,j)=>[t,quote(result.time_unit),result.sides[i]===1?'post':'pre',quote(name),quote(result.units[j]),result.columns[j][i]].join(','))).join('\n')+'\n';
  $('download').disabled=false;
}
async function run(event) {
  event.preventDefault();if(running||!$('inputs').reportValidity())return;
  const mode=$('model-picker').value,guided=mode==='synthetic';
  try {
    await ready;
    let source,request,label;
    if(guided){
      const [dose,clearance,volume]=inputs.map(id=>Number($(id).value));
      if(![dose,clearance,volume].every(Number.isFinite))throw new Error('Enter finite parameter values.');
      source=baseline.source.replace('cl = 0.3 [L/h]',`cl = ${clearance} [L/h]`).replace('v = 3 [L]',`v = ${volume} [L]`);
      request=structuredClone(baseline.request);request.regimen.administrations[0].amount.value=dose;
      request.regimen.observations=Array.from({length:97},(_,i)=>({time:{value:i/4,unit:'h'},side:'post'}));
      label=`${dose} mg, clearance ${clearance} L/h, volume ${volume} L`;
    }else{
      source=$('source').value;request=JSON.parse($('request').value);label=mode==='custom'?'your edited model/request':$('model-picker').selectedOptions[0].textContent;
      if(mode.startsWith('library-')&&!$('library-controls').hidden)label+=` · ${$('parameter').selectedOptions[0].textContent} = ${$('parameter-value').value}`;
    }
    if(!source.trim()||byteLength(source)>1000000||byteLength(JSON.stringify(request))>1000000)throw new Error('Provide a model and request of at most one million bytes each.');
    if(request?.schema!=='qspquest.run/v0.1')throw new Error('Use a qspquest.run/v0.1 simulation request.');
    if(!request.budgets||request.budgets.output_values>20000||request.budgets.solver_callbacks>1000000||request.budgets.events>10000)throw new Error('Browser limits: output_values ≤ 20000, solver_callbacks ≤ 1000000, events ≤ 10000. Reduce the request budgets.');
    const prior=current;clearResult();busy(true);state('Loading WebAssembly and simulating on your device…');
    const {result}=await client.run({kind:'run',source,request},{timeoutMs:30000});
    if(!result.times.length||result.times.length>20000||!result.names.length||result.columns.some(v=>v.length!==result.times.length||!Array.from(v).every(Number.isFinite)))throw new Error('The engine returned an invalid or oversized trajectory.');
    if(guided){const [dose,clearance,volume]=inputs.map(id=>Number($(id).value));const error=Math.max(...Array.from(result.times,(t,i)=>Math.abs(result.columns[0][i]-dose/volume*Math.exp(-clearance/volume*t))));if(error>1e-7*Math.max(1,dose/volume))throw new Error('Simulation differs from the analytic reference.');}
    lastResult=prior||lastResult;current={result,key:JSON.stringify({source,request}),modelKey:mode,label,outputLabels:mode.startsWith('library-')?library[Number(mode.slice(8))].output_labels:{}};
    $('output').replaceChildren();result.names.forEach((name,i)=>{const option=document.createElement('option');option.value=i;option.textContent=`${current.outputLabels?.[name]||name} (${result.units[i]})`;$('output').append(option);});
    render(0);state(guided?'Complete. Numerical results match the analytic reference.':'Complete. Plot an output or download all numerical results.');
  }catch(error){state(`${error.code||'simulation_error'}: ${error.message}`,true);}
  finally{busy(false);}
}
$('inputs').addEventListener('submit',run);$('cancel').onclick=()=>client.cancel();
$('parameter').onchange=selectParameter;
$('parameter-value').oninput=()=>{
  if(current)lastResult=current;clearResult();
  const value=Number($('parameter-value').value);
  if(!Number.isFinite(value)||!$('parameter-value').value.trim()){state('Enter a finite parameter value.',true);return;}
  try{const request=JSON.parse($('request').value);request.parameters[$('parameter').value].value=value;$('request').value=JSON.stringify(request,null,2);state('Parameter changed. Run simulation to compare the result.');}catch(error){state(error.message,true);}
};
$('model-picker').onchange=selectModel;$('output').onchange=()=>render(Number($('output').value));
$('restore').onclick=()=>{$('inputs').reset();$('model-picker').value='synthetic';lastResult=undefined;selectModel();state('Example restored. Select Run simulation to start.');};
for(const id of [...inputs,'source','request'])$(id).addEventListener('input',()=>{if(current)lastResult=current;clearResult();if(id==='source'||id==='request')refreshParameters();state('Inputs changed. Run simulation to update the results.');});
for(const [id,target]of[['model-file','source'],['request-file','request']])$(id).onchange=async()=>{
  const file=$(id).files[0];if(!file)return;
  if(file.size>1000000){state('The file exceeds one million bytes.',true);return;}
  try{$(target).value=await file.text();if(current)lastResult=current;clearResult();refreshParameters();state(`Loaded ${file.name} locally. Inspect inputs, then run simulation.`);}catch(error){state(error.message,true);}
};
$('download').onclick=()=>{if(!csv)return;if(downloadUrl)URL.revokeObjectURL(downloadUrl);downloadUrl=URL.createObjectURL(new Blob([csv],{type:'text/csv;charset=utf-8'}));const link=document.createElement('a');link.href=downloadUrl;link.download='qspquest-simulation.csv';link.click();};
window.addEventListener('pagehide',()=>{client.dispose();if(downloadUrl)URL.revokeObjectURL(downloadUrl);});
