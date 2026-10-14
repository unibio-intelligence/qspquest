export type WorkerLike = Pick<Worker, 'postMessage' | 'terminate' | 'onmessage' | 'onerror' | 'onmessageerror'>;
export interface ColumnarTrajectory {
  times: Float64Array;
  /** 0 = pre-event, 1 = post-event. */
  sides: Uint8Array;
  columns: Float64Array[];
}
export interface NamedTrajectory extends ColumnarTrajectory {
  names: string[];
  units: string[];
}
/** Draft text-model payload; times/amounts are in the model's declared units. */
export interface TextSimulationPayload {
  kind: 'text';
  source: string;
  overrides?: Record<string, {value: number; unit: string}>;
  protocol: {
    start?: number; end: number; samples: number[]; rtol: number; atol: number;
    events: Array<{kind: 'bolus'; time: number; target: number; amount: number}
      | {kind: 'infusion'; time: number; target: number; amount: number; duration: number}
      | {kind: 'reset'; time: number; target: number; value: number; order: number; active_inputs: 'continue' | 'stop_target'}>;
  };
  callbacks?: number;
}
export interface FixedAdministration {
  target: string;
  time: {value: number; unit: string};
  amount: {value: number; unit: string};
  delivery: {kind: 'bolus'} | {kind: 'infusion'; span:
    {kind: 'duration'; duration: {value: number; unit: string}} |
    {kind: 'rate'; rate: {value: number; unit: string}}};
  repeat?: {interval: {value: number; unit: string}; additional: number} | null;
  lag?: {value: number; unit: string} | null;
  bioavailability: number;
}
export interface RegimenSimulationPayload extends Omit<TextSimulationPayload, 'protocol'> {
  regimen: {
    start?: {value: number; unit: string};
    end: {value: number; unit: string};
    samples: Array<{value: number; unit: string}>;
    checkpoints?: Array<{value: number; unit: string}>;
    observations?: Array<{time: {value: number; unit: string}; side: 'pre' | 'post'}> | null;
    administrations: FixedAdministration[];
    covariates?: Record<string, {value: number; unit: string}>;
    covariate_changes?: Array<{name: string; time: {value: number; unit: string}; value: {value: number; unit: string}}>;
    resets?: Array<{target: string; time: {value: number; unit: string};
      value: {value: number; unit: string}; order: number; active_inputs: 'continue' | 'stop_target'}>;
    rtol: number;
    /** Legacy scalar in each state's unit. Supply exactly one tolerance mode. */
    atol?: number | null;
    absolute_tolerances?: Record<string, {value: number; unit: string}> | null;
  };
}
export interface ScientificDiagnostic {
  code: string;
  message: string;
  time?: number | null;
  expression?: string | null;
  line?: number;
  column?: number;
  hint?: string;
}
export type WorkerRequest<T = unknown> = {type: 'run'; requestId: number; payload: T};
export type WorkerReply<T = ColumnarTrajectory> =
  | {type: 'started'; requestId: number}
  | {type: 'result'; requestId: number; result: T}
  | {type: 'error'; requestId: number; error: ScientificDiagnostic};
export class WorkerClient {
  constructor(createWorker: () => WorkerLike);
  run<T = ColumnarTrajectory>(payload: unknown, options?: {timeoutMs?: number; onStarted?: (requestId: number) => void}): Promise<{requestId: number; result: T}>;
  cancel(code?: string): void;
  dispose(): void;
}

export interface ScientificRunPayload {
  kind: 'run';
  source: string;
  request: {
    schema: 'qspquest.run/v0.1';
    request_id?: string | null;
    /** Seeded stochastic execution is currently rejected. */
    seed?: null;
    solver: 'diffsol_bdf';
    budgets: {solver_callbacks: number; output_values: number; events: number};
    parameters?: Record<string, {value: number; unit: string}>;
    initial_states?: Record<string, {value: number; unit: string}>;
    regimen: RegimenSimulationPayload['regimen'];
  };
}
export interface ScientificTrajectory extends NamedTrajectory {
  schema: 'qspquest.result/v0.1';
  request_id: string | null;
  model_id: string;
  description: string;
  time_unit: string;
  identity: {
    model_content_hash: string; spec_version: string; compiler_version: string;
    compiler_source_sha256: string; rustc_version: string; target: string;
    build_profile: string; rustflags_sha256: string; bound_value_hash: string;
    run_request_hash: string; backend: 'diffsol'; backend_version: string;
    algorithm: 'bdf';
  };
}
