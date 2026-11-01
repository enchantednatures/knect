// Typed JSON-RPC client for `POST /rpc`. One in-flight request per call, with
// request IDs, timeout, and caller-supplied cancellation.

import type {
  ActionResult,
  DaemonStatus,
  FlaggerControlParams,
  GetDiffParams,
  GetNodeDetailsParams,
  GetTopologyParams,
  MutationDiff,
  NodeDetails,
  RpcErrorPayload,
  RpcId,
  RpcRequest,
  RpcResponse,
  SetTrafficSplitParams,
  TapHandle,
  TapStartParams,
  Topology,
  ValidationResult,
  ValidateManifestParams,
} from './types';
import { isRecord } from './types';

export const RPC_TIMEOUT = -32011;

export class RpcCallError extends Error {
  readonly code: number;
  readonly data: RpcErrorPayload['data'];
  readonly httpStatus: number | null;

  constructor(
    code: number,
    message: string,
    data: RpcErrorPayload['data'] = null,
    httpStatus: number | null = null,
  ) {
    super(message);
    this.name = 'RpcCallError';
    this.code = code;
    this.data = data;
    this.httpStatus = httpStatus;
  }
}

export interface RpcCallOptions {
  timeoutMs?: number;
  signal?: AbortSignal;
}

export interface RpcClientOptions {
  /** Base URL of the daemon, e.g. `http://127.0.0.1:7420`. Empty means same origin. */
  baseUrl?: string;
  fetchImpl?: typeof fetch;
  timeoutMs?: number;
  /** Injectable ID source, mainly for deterministic tests. */
  idFactory?: () => RpcId;
}

export class RpcClient {
  private readonly baseUrl: string;
  private readonly fetchImpl: typeof fetch;
  private readonly timeoutMs: number;
  private readonly idFactory: () => RpcId;
  private nextId = 1;

  constructor(options: RpcClientOptions = {}) {
    this.baseUrl = (options.baseUrl ?? '').replace(/\/+$/, '');
    this.fetchImpl = options.fetchImpl ?? globalThis.fetch.bind(globalThis);
    this.timeoutMs = options.timeoutMs ?? 15_000;
    this.idFactory = options.idFactory ?? (() => this.nextId++);
  }

  get endpoint(): string {
    return `${this.baseUrl}/rpc`;
  }

  async call<T>(method: string, params?: unknown, options: RpcCallOptions = {}): Promise<T> {
    const id = this.idFactory();
    const payload: RpcRequest = { jsonrpc: '2.0', id, method };
    if (params !== undefined) {
      payload.params = params;
    }

    const controller = new AbortController();
    let timedOut = false;
    const timeoutMs = options.timeoutMs ?? this.timeoutMs;
    const timer = setTimeout(() => {
      timedOut = true;
      controller.abort();
    }, timeoutMs);
    const onExternalAbort = (): void => controller.abort();
    if (options.signal?.aborted) {
      controller.abort();
    } else {
      options.signal?.addEventListener('abort', onExternalAbort, { once: true });
    }

    let response: Response;
    try {
      response = await this.fetchImpl(this.endpoint, {
        method: 'POST',
        headers: {
          'content-type': 'application/json',
          accept: 'application/json',
        },
        body: JSON.stringify(payload),
        signal: controller.signal,
      });
    } catch (error) {
      if (options.signal?.aborted) {
        throw new RpcCallError(-32012, `rpc ${method} was cancelled`);
      }
      if (timedOut) {
        throw new RpcCallError(RPC_TIMEOUT, `rpc ${method} timed out after ${timeoutMs}ms`);
      }
      const detail = error instanceof Error ? error.message : String(error);
      throw new RpcCallError(-32603, `rpc ${method} failed: ${detail}`);
    } finally {
      clearTimeout(timer);
      options.signal?.removeEventListener('abort', onExternalAbort);
    }

    let body: unknown;
    try {
      body = await response.json();
    } catch {
      throw new RpcCallError(
        -32700,
        `rpc ${method} returned a non-JSON response (HTTP ${response.status})`,
        null,
        response.status,
      );
    }

    if (!isRecord(body)) {
      throw new RpcCallError(-32603, `rpc ${method} returned an invalid envelope`, null, response.status);
    }

    const envelope = body as unknown as RpcResponse<T>;
    if (envelope.error) {
      throw new RpcCallError(
        envelope.error.code,
        envelope.error.message,
        envelope.error.data ?? null,
        response.status,
      );
    }
    if (!response.ok) {
      throw new RpcCallError(
        -32603,
        `rpc ${method} failed with HTTP ${response.status}`,
        null,
        response.status,
      );
    }
    return (envelope.result ?? null) as T;
  }

  status(): Promise<DaemonStatus> {
    return this.call<DaemonStatus>('daemon.status', {});
  }

  getTopology(params: GetTopologyParams = {}): Promise<Topology> {
    return this.call<Topology>('graph.get_topology', params);
  }

  getNodeDetails(id: GetNodeDetailsParams['id'], includeYaml = true): Promise<NodeDetails> {
    const params: GetNodeDetailsParams = { id, include_yaml: includeYaml };
    return this.call<NodeDetails>('graph.get_node_details', params);
  }

  getDiff(id: GetDiffParams['id'], mode: GetDiffParams['mode'] = 'three_way'): Promise<MutationDiff> {
    return this.call<MutationDiff>('mutations.get_diff', { id, mode } satisfies GetDiffParams);
  }

  refresh(rediscover = false): Promise<{ generation: number }> {
    return this.call<{ generation: number }>('graph.refresh', { rediscover });
  }

  tapList(): Promise<TapHandle[]> {
    return this.call<TapHandle[]>('tap.list', {});
  }

  tapStart(params: TapStartParams): Promise<TapHandle> {
    return this.call<TapHandle>('tap.start', params);
  }

  tapStop(tapId: string): Promise<{ removed: boolean }> {
    return this.call<{ removed: boolean }>('tap.stop', { tap_id: tapId });
  }

  setTrafficSplit(params: SetTrafficSplitParams): Promise<ActionResult> {
    return this.call<ActionResult>('actions.set_traffic_split', params);
  }

  flaggerControl(params: FlaggerControlParams): Promise<ActionResult> {
    return this.call<ActionResult>('actions.flagger_control', params);
  }

  validateManifest(params: ValidateManifestParams): Promise<ValidationResult> {
    return this.call<ValidationResult>('policy.validate_manifest', params);
  }
}
