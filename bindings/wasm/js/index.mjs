import { WorkerClient } from './client.mjs';
export { WorkerClient } from './client.mjs';

/** Instantiate lazily on the browser main thread, after feature/auth gating. */
export function createEngineClient() {
  return new WorkerClient(() => new Worker(new URL('./engine-worker.mjs', import.meta.url), { type: 'module' }));
}
