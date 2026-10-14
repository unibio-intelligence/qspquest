import test from 'node:test';
import assert from 'node:assert/strict';
import { WorkerClient } from './client.mjs';
function harness() {
  const workers = [];
  const client = new WorkerClient(() => {
    const worker = { terminated: false, messages: [], postMessage(x) { this.messages.push(x); }, terminate() { this.terminated = true; } };
    workers.push(worker); return worker;
  });
  return { client, workers };
}
test('queued old-worker messages cannot resolve a newer request', async () => {
  const {client, workers} = harness();
  const old = client.run({edit: 1});
  const rejection = assert.rejects(old, {code:'superseded'});
  const stale = workers[0].onmessage;
  const current = client.run({edit: 2});
  assert.equal(workers[0].terminated, true);
  stale({data:{type:'result',requestId:2,result:'stale'}});
  workers[1].onmessage({data:{type:'result',requestId:1,result:'wrong request'}});
  workers[1].onmessage({data:{type:'result',requestId:2,result:'latest'}});
  assert.deepEqual(await current, {requestId:2,result:'latest'});
  await rejection; client.dispose();
});
test('timeout terminates the worker and the next request can recover', async () => {
  const {client, workers} = harness();
  await assert.rejects(client.run({}, {timeoutMs:5}), {code:'time_budget'});
  assert.equal(workers[0].terminated,true);
  const next = client.run({});
  workers[1].onmessage({data:{type:'result',requestId:2,result:42}});
  assert.equal((await next).result,42);client.dispose();
});
test('dispose rejects active work and prevents reuse', async () => {
  const {client,workers} = harness(); const active=client.run({});
  client.dispose();await assert.rejects(active,{code:'disposed'});
  await assert.rejects(client.run({}),{code:'disposed'});
  assert.equal(workers.length,1);assert.equal(workers[0].terminated,true);
});
test('startup and transport failures release the request', async () => {
  const client=new WorkerClient(()=>{throw new Error('creation failed');});
  await assert.rejects(client.run({}),{code:'worker_failure'});client.dispose();
  const h=harness();const active=h.client.run({});
  h.workers[0].onmessageerror();await assert.rejects(active,{code:'worker_protocol'});
  assert.equal(h.workers[0].terminated,true);h.client.dispose();
});

// Run the worker's admission and lifetime contract without requiring a Wasm build.
async function workerHarness() {
  const { readFile } = await import('node:fs/promises');
  const { runInNewContext } = await import('node:vm');
  const source = await readFile(new URL('./engine-worker.mjs', import.meta.url), 'utf8');
  const messages = [], compiled = [];
  class Simulation {
    constructor(text) { this.source = text; this.freed = false; compiled.push(this); }
    static from_text(text) { return new Simulation(text); }
    free() { this.freed = true; }
    execute_selected() {
      assert.equal(this.freed, false);
      return { metadata_json: () => '{}', times: () => new Float64Array(),
        sides: () => new Uint8Array(), state_count: () => 0, free() {} };
    }
  }
  const self = { postMessage(message) { messages.push(message); } };
  runInNewContext(source.replace(/^import .*;$/m, ''), {
    self, TextEncoder, init: async () => {}, CompiledSimulation: Simulation
  });
  let id = 0;
  return { compiled, async run(payload) {
    messages.length = 0;
    await self.onmessage({ data: { type: 'run', requestId: ++id, payload } });
    return messages.at(-1);
  } };
}
test('worker admits model bytes separately from request bytes', async () => {
  const h = await workerHarness();
  const payload = { kind: 'run', source: 'x'.repeat(3000000), request: {} };
  assert.equal((await h.run(payload)).type, 'result');
  assert.equal((await h.run({ ...payload, source: payload.source + 'x' })).error.code, 'invalid_input');
  assert.equal((await h.run({ ...payload, source: 'é'.repeat(1500001) })).error.code, 'invalid_input');
  const request = { x: 'x'.repeat(999992) };
  assert.equal(new TextEncoder().encode(JSON.stringify(request)).length, 1000000);
  assert.equal((await h.run({ ...payload, request })).type, 'result');
  assert.equal((await h.run({ ...payload, request: { x: request.x + 'x' } })).error.code, 'invalid_input');
  assert.equal(h.compiled.length, 1);
});
test('worker reuses a compiled source and releases it only after replacement', async () => {
  const h = await workerHarness();
  const payload = { kind: 'run', source: 'first model', request: {} };
  assert.equal((await h.run(payload)).type, 'result');
  assert.equal((await h.run({ ...payload, request: { parameters: { cl: 2 } } })).type, 'result');
  assert.equal(h.compiled.length, 1);
  assert.equal(h.compiled[0].freed, false);
  assert.equal((await h.run({ ...payload, source: 'second model' })).type, 'result');
  assert.equal(h.compiled.length, 2);
  assert.equal(h.compiled[0].freed, true);
  assert.equal(h.compiled[1].freed, false);
});
