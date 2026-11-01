import { describe, expect, it, vi } from 'vitest';
import { EventStream, parseSseEvent, type EventSourceLike } from '../lib/events';
import type { SseEvent } from '../lib/types';
import { topology } from './fixtures';

class FakeSource implements EventSourceLike {
  onopen: ((event: Event) => void) | null = null;
  onerror: ((event: Event) => void) | null = null;
  closed = false;
  readonly listeners = new Map<string, (event: MessageEvent<string>) => void>();

  constructor(readonly url: string) {}

  addEventListener(type: string, listener: (event: MessageEvent<string>) => void): void {
    this.listeners.set(type, listener);
  }

  close(): void {
    this.closed = true;
  }

  open(): void {
    this.onopen?.(new Event('open'));
  }

  fail(): void {
    this.onerror?.(new Event('error'));
  }

  emit(name: string, payload: unknown, lastEventId = ''): void {
    const listener = this.listeners.get(name);
    if (!listener) throw new Error(`no listener for ${name}`);
    listener({ data: JSON.stringify(payload), lastEventId } as MessageEvent<string>);
  }
}

function snapshotEvent(seq = 42): SseEvent {
  return { seq, ts: '2026-10-31T12:00:00Z', kind: 'snapshot', data: topology };
}

describe('parseSseEvent', () => {
  it('parses a valid frame and rejects malformed or mismatched frames', () => {
    const event = snapshotEvent();
    expect(parseSseEvent('snapshot', JSON.stringify(event))).toEqual(event);
    expect(parseSseEvent('graph.patch', JSON.stringify(event))).toBeNull();
    expect(parseSseEvent('snapshot', '{')).toBeNull();
    expect(parseSseEvent('snapshot', JSON.stringify({ seq: 1, ts: 'x', kind: 'snapshot' }))).toBeNull();
  });
});

describe('EventStream', () => {
  it('delivers named events and persists the last sequence', () => {
    const sources: FakeSource[] = [];
    const storage = new Map<string, string>();
    const received: SseEvent[] = [];
    const statuses: string[] = [];
    const stream = new EventStream({
      baseUrl: 'http://daemon.test',
      onEvent: (event) => received.push(event),
      onStatus: (state) => statuses.push(state),
      eventSourceFactory: (url) => {
        const source = new FakeSource(url);
        sources.push(source);
        return source;
      },
      storage: {
        getItem: (key) => storage.get(key) ?? null,
        setItem: (key, value) => storage.set(key, value),
      },
    });

    stream.connect();
    expect(sources[0]?.url).toBe('http://daemon.test/events');
    sources[0]?.open();
    sources[0]?.emit('snapshot', snapshotEvent(42), '42');

    expect(statuses).toContain('live');
    expect(received).toHaveLength(1);
    expect(storage.get('knect.webui.last-event-id')).toBe('42');

    stream.restart();
    expect(sources[1]?.url).toBe('http://daemon.test/events?last_event_id=42');
    stream.close();
    expect(statuses.at(-1)).toBe('closed');
  });

  it('ignores stale non-snapshot events after a newer sequence was applied', () => {
    const sources: FakeSource[] = [];
    const received: SseEvent[] = [];
    const stream = new EventStream({
      onEvent: (event) => received.push(event),
      eventSourceFactory: (url) => {
        const source = new FakeSource(url);
        sources.push(source);
        return source;
      },
      storage: null,
    });
    stream.connect();
    sources[0]?.emit('snapshot', snapshotEvent(10), '10');
    sources[0]?.emit('daemon.heartbeat', {
      seq: 8,
      ts: '2026-10-31T12:00:00Z',
      kind: 'daemon.heartbeat',
      data: { uptime_secs: 1, generation: 1, node_count: 0, edge_count: 0, ts: 'x' },
    }, '8');
    expect(received).toHaveLength(1);
    stream.close();
  });

  it('reconnects after a stream error with backoff', () => {
    vi.useFakeTimers();
    const sources: FakeSource[] = [];
    const statuses: string[] = [];
    const stream = new EventStream({
      onEvent: () => {},
      onStatus: (state) => statuses.push(state),
      eventSourceFactory: (url) => {
        const source = new FakeSource(url);
        sources.push(source);
        return source;
      },
      storage: null,
      reconnectMaxMs: 100,
    });
    stream.connect();
    sources[0]?.fail();
    expect(sources[0]?.closed).toBe(false);
    expect(statuses).toContain('reconnecting');
    vi.advanceTimersByTime(100);
    expect(sources[0]?.closed).toBe(true);
    expect(sources).toHaveLength(2);
    stream.close();
    vi.useRealTimers();
  });
});
