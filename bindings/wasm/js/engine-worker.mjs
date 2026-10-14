// Public simulation contract. Conformance probes are intentionally absent.
import init, { CompiledSimulation } from '../pkg/engine/qspquest_wasm.js';
let ready;
let cachedSource;
let compiled;

self.onmessage = async ({ data }) => {
  if (data?.type !== 'run' || !Number.isSafeInteger(data.requestId) || data.requestId <= 0) return;
  const { requestId, payload } = data;
  let trajectory;
  try {
    if (!payload || payload.kind !== 'run' || typeof payload.source !== 'string' ||
        !payload.request || Object.keys(payload).some(key => !['kind', 'source', 'request', 'outputs'].includes(key))) {
      throw { code: 'invalid_input', message: 'Provide a model source and a versioned simulation request' };
    }
    if (new TextEncoder().encode(payload.source).length > 3000000) {
      throw { code: 'invalid_input', message: 'Model source must fit within three million bytes' };
    }
    if (new TextEncoder().encode(JSON.stringify(payload.request)).length > 1000000) {
      throw { code: 'invalid_input', message: 'Simulation request must fit within one million bytes' };
    }
    if (payload.outputs !== undefined && (!Array.isArray(payload.outputs) ||
        payload.outputs.length === 0 || payload.outputs.some(name => typeof name !== 'string') ||
        new TextEncoder().encode(JSON.stringify(payload.outputs)).length > 1000000)) {
      throw { code: 'invalid_input', message: 'Select a nonempty array of output names within one million bytes' };
    }
    ready ??= init();
    await ready;
    if (cachedSource !== payload.source) {
      const next = payload.source.trimStart().startsWith('{')
        ? new CompiledSimulation(payload.source)
        : CompiledSimulation.from_text(payload.source);
      compiled?.free();
      compiled = next;
      cachedSource = payload.source;
    }
    self.postMessage({ type: 'started', requestId });
    trajectory = compiled.execute_selected(JSON.stringify(payload.request), JSON.stringify(payload.outputs ?? null));
    const result = {
      ...JSON.parse(trajectory.metadata_json()),
      times: trajectory.times(), sides: trajectory.sides(),
      columns: Array.from({ length: trajectory.state_count() }, (_, i) => trajectory.column(i))
    };
    self.postMessage({ type: 'result', requestId, result },
      [result.times.buffer, result.sides.buffer, ...result.columns.map(column => column.buffer)]);
  } catch (error) {
    let diagnostic;
    if (error && typeof error === 'object' && typeof error.code === 'string') diagnostic = error;
    else {
      try { diagnostic = JSON.parse(String(error)); }
      catch { diagnostic = { code: 'worker_failure', message: String(error) }; }
    }
    self.postMessage({ type: 'error', requestId, error: diagnostic });
  } finally {
    trajectory?.free();
  }
};
