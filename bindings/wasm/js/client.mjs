/** A single-flight worker owner. Each edit supersedes the previous request. */
export class WorkerClient {
  constructor(createWorker) {
    this.createWorker = createWorker;
    this.worker = null;
    this.generation = 0;
    this.sequence = 0;
    this.pending = null;
    this.disposed = false;
  }

  run(payload, { timeoutMs = 30000, onStarted } = {}) {
    if (this.disposed) return Promise.reject(failure('disposed', 'Worker client is disposed'));
    if (!Number.isFinite(timeoutMs) || timeoutMs <= 0 || timeoutMs > 2147483647) {
      return Promise.reject(failure('invalid_input', 'timeoutMs must be a positive timer interval'));
    }
    if (!Number.isSafeInteger(this.sequence + 1)) {
      return Promise.reject(failure('request_limit', 'Request sequence exhausted'));
    }
    this.cancel('superseded');
    const requestId = ++this.sequence;
    return new Promise((resolve, reject) => {
      this.pending = { requestId, resolve, reject, onStarted, timer: null };
      try {
        if (!this.worker) this.startWorker();
        this.pending.timer = setTimeout(() => this.cancel('time_budget'), timeoutMs);
        this.worker.postMessage({ type: 'run', requestId, payload });
      } catch (error) {
        this.fail('worker_failure', String(error));
      }
    });
  }

  startWorker() {
    const generation = ++this.generation;
    const worker = this.createWorker();
    this.worker = worker;
    worker.onmessage = ({ data }) => {
      // A terminated worker can already have a delivery queued on the main thread.
      if (generation !== this.generation || worker !== this.worker) return;
      const pending = this.pending;
      if (!pending || data?.requestId !== pending.requestId) return;
      if (data.type === 'started') {
        // Observer callbacks must not strand an execution if UI code throws.
        try { pending.onStarted?.(pending.requestId); } catch { /* notification only */ }
        return;
      }
      if (data.type !== 'result' && data.type !== 'error') {
        this.fail('worker_protocol', 'Unexpected worker message');
        return;
      }
      clearTimeout(pending.timer);
      this.pending = null;
      if (data.type === 'error') {
        const error = data.error;
        pending.reject(failure(typeof error?.code === 'string' ? error.code : 'worker_failure',
          typeof error?.message === 'string' ? error.message : 'Worker failed', error));
      } else {
        pending.resolve({ requestId: pending.requestId, result: data.result });
      }
    };
    worker.onerror = event => {
      if (generation !== this.generation || worker !== this.worker) return;
      this.fail('worker_failure', event.message || 'Worker failed');
    };
    worker.onmessageerror = () => {
      if (generation !== this.generation || worker !== this.worker) return;
      this.fail('worker_protocol', 'Worker result could not be decoded');
    };
  }

  cancel(code = 'cancelled') {
    if (!this.pending) return;
    this.fail(code, code === 'superseded' ? 'A newer request replaced this run' : 'Run cancelled');
  }

  fail(code, message) {
    const pending = this.pending;
    this.pending = null;
    ++this.generation;
    if (this.worker) this.worker.terminate();
    this.worker = null;
    if (pending) {
      clearTimeout(pending.timer);
      pending.reject(failure(code, message));
    }
  }

  dispose() {
    this.disposed = true;
    this.fail('disposed', 'Worker client is disposed');
  }
}

function failure(code, message, diagnostic) {
  return Object.assign(new Error(message), { code, diagnostic });
}
