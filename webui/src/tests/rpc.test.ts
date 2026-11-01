import { describe, expect, it, vi } from 'vitest';
import { RpcCallError, RpcClient } from '../lib/rpc';

function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

function typedFetch(
  impl: (input: RequestInfo | URL, init?: RequestInit) => Promise<Response>,
): typeof fetch {
  return vi.fn(impl) as unknown as typeof fetch;
}

describe('RpcClient', () => {
  it('posts a JSON-RPC envelope and returns the result', async () => {
    const mock = vi.fn(async (_input: RequestInfo | URL, _init?: RequestInit) =>
      json({
        jsonrpc: '2.0',
        id: 1,
        result: { schema_version: 1, context: 'fixture', generation: 7 },
        error: null,
      }),
    );
    const fetchImpl = mock as unknown as typeof fetch;
    const client = new RpcClient({ fetchImpl, baseUrl: 'http://daemon.test/' });

    const result = await client.getTopology({ min_generation: 7 });

    expect(result).toMatchObject({ context: 'fixture', generation: 7 });
    expect(mock).toHaveBeenCalledTimes(1);
    const [url, init] = mock.mock.calls[0]!;
    expect(url).toBe('http://daemon.test/rpc');
    expect(init?.method).toBe('POST');
    const body = JSON.parse(String(init?.body)) as Record<string, unknown>;
    expect(body['jsonrpc']).toBe('2.0');
    expect(body['id']).toBe(1);
    expect(body['method']).toBe('graph.get_topology');
    expect(body['params']).toEqual({ min_generation: 7 });
  });

  it('issues monotonically increasing request IDs and omits undefined params', async () => {
    const mock = vi.fn(async (_input: RequestInfo | URL, _init?: RequestInit) =>
      json({ jsonrpc: '2.0', id: 1, result: {}, error: null }),
    );
    const fetchImpl = mock as unknown as typeof fetch;
    const client = new RpcClient({ fetchImpl });

    await client.status();
    await client.tapList();

    const first = JSON.parse(String(mock.mock.calls[0]?.[1]?.body)) as Record<string, unknown>;
    const second = JSON.parse(String(mock.mock.calls[1]?.[1]?.body)) as Record<string, unknown>;
    expect(first['id']).toBe(1);
    expect(second['id']).toBe(2);
    expect(first['method']).toBe('daemon.status');
    expect(second['method']).toBe('tap.list');
  });

  it('surfaces a JSON-RPC error with code and data', async () => {
    const fetchImpl = typedFetch(async () =>
      json({
        jsonrpc: '2.0',
        id: 1,
        result: null,
        error: {
          code: -32001,
          message: 'resource not found',
          data: { retryable: false, details: 'Revision missing', resource: null },
        },
      }),
    );
    const client = new RpcClient({ fetchImpl });

    const failure = client.call('graph.get_node_details', { id: 'x' });
    await expect(failure).rejects.toBeInstanceOf(RpcCallError);
    await failure.catch((error: RpcCallError) => {
      expect(error.code).toBe(-32001);
      expect(error.message).toBe('resource not found');
      expect(error.data?.details).toBe('Revision missing');
    });
  });

  it('rejects a non-JSON response', async () => {
    const fetchImpl = typedFetch(async () => new Response('not json', { status: 200 }));
    const client = new RpcClient({ fetchImpl });

    await expect(client.status()).rejects.toMatchObject({ code: -32700 });
  });

  it('rejects an HTTP error even when the envelope is empty', async () => {
    const fetchImpl = typedFetch(async () => json({ jsonrpc: '2.0', id: 1 }, 500));
    const client = new RpcClient({ fetchImpl });

    await expect(client.status()).rejects.toMatchObject({ code: -32603, httpStatus: 500 });
  });

  it('times out a hung request with the protocol timeout code', async () => {
    const fetchImpl = typedFetch(
      (_input, init) =>
        new Promise<Response>((_resolve, reject) => {
          init?.signal?.addEventListener('abort', () => {
            reject(new DOMException('aborted', 'AbortError'));
          });
        }),
    );
    const client = new RpcClient({ fetchImpl, timeoutMs: 5 });

    await expect(client.status()).rejects.toMatchObject({ code: -32011 });
  });

  it('honours an external AbortSignal with the cancelled code', async () => {
    const fetchImpl = typedFetch(
      (_input, init) =>
        new Promise<Response>((_resolve, reject) => {
          if (init?.signal?.aborted) {
            reject(new DOMException('aborted', 'AbortError'));
            return;
          }
          init?.signal?.addEventListener('abort', () => {
            reject(new DOMException('aborted', 'AbortError'));
          });
        }),
    );
    const client = new RpcClient({ fetchImpl, timeoutMs: 1_000 });
    const controller = new AbortController();
    controller.abort();

    await expect(
      client.call('daemon.status', {}, { signal: controller.signal }),
    ).rejects.toMatchObject({ code: -32012 });
  });
});
