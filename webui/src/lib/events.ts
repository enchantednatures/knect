// SSE client for `GET /events`.
//
// The daemon replays from `Last-Event-ID` and falls back to a fresh `snapshot`
// when the replay ring has rolled over. Browsers cannot set that header on a
// re-created EventSource, so this client persists the last sequence it applied
// and, on reconnect, asks for it via `?last_event_id=` (forward compatible)
// while treating the daemon's snapshot as the authoritative recovery path.

import { SSE_EVENT_NAMES, isRecord, type SseEvent } from './types';

export type ConnectionState = 'idle' | 'connecting' | 'live' | 'reconnecting' | 'closed' | 'error';

export interface EventSourceLike {
  onopen: ((event: Event) => void) | null;
  onerror: ((event: Event) => void) | null;
  addEventListener(type: string, listener: (event: MessageEvent<string>) => void): void;
  close(): void;
}

export interface EventStreamOptions {
  baseUrl?: string;
  onEvent: (event: SseEvent) => void;
  onStatus?: (state: ConnectionState) => void;
  onError?: (error: unknown) => void;
  eventSourceFactory?: (url: string) => EventSourceLike;
  storage?: Pick<Storage, 'getItem' | 'setItem'> | null;
  /** Base reconnect delay. Exposed for tests. */
  reconnectBaseMs?: number;
  reconnectMaxMs?: number;
  /** A silent stream is restarted after this long. The daemon heartbeats every 15s. */
  staleAfterMs?: number;
  now?: () => number;
}

const LAST_EVENT_KEY = 'knect.webui.last-event-id';

export function parseSseEvent(name: string, payload: string): SseEvent | null {
  let value: unknown;
  try {
    value = JSON.parse(payload);
  } catch {
    return null;
  }
  if (!isRecord(value)) return null;
  if (value['kind'] !== name) return null;
  if (typeof value['seq'] !== 'number' || typeof value['ts'] !== 'string') return null;
  if (!('data' in value)) return null;
  return value as unknown as SseEvent;
}

export class EventStream {
  private readonly baseUrl: string;
  private readonly options: EventStreamOptions;
  private source: EventSourceLike | null = null;
  private reconnectTimer: ReturnType<typeof setTimeout> | null = null;
  private watchdogTimer: ReturnType<typeof setInterval> | null = null;
  private attempt = 0;
  private stopped = true;
  private lastEventAt = 0;
  private applied: SseEvent | null = null;

  constructor(options: EventStreamOptions) {
    this.options = options;
    this.baseUrl = (options.baseUrl ?? '').replace(/\/+$/, '');
  }

  private get url(): string {
    const id = this.applied?.seq ?? this.lastEventId;
    const query = id === null ? '' : `?last_event_id=${id}`;
    return `${this.baseUrl}/events${query}`;
  }

  get lastEventId(): number | null {
    const raw = this.options.storage?.getItem(LAST_EVENT_KEY) ?? null;
    if (raw === null) return null;
    const parsed = Number(raw);
    return Number.isSafeInteger(parsed) ? parsed : null;
  }

  get lastApplied(): SseEvent | null {
    return this.applied;
  }

  connect(): void {
    this.stopped = false;
    this.open();
    this.startWatchdog();
  }

  close(): void {
    this.stopped = true;
    this.clearTimers();
    this.source?.close();
    this.source = null;
    this.setStatus('closed');
  }

  /** Force a fresh connection; used by the stale-stream watchdog. */
  restart(): void {
    if (this.stopped) return;
    this.clearReconnectTimer();
    this.source?.close();
    this.source = null;
    this.open();
  }

  private open(): void {
    this.setStatus(this.attempt === 0 ? 'connecting' : 'reconnecting');
    const factory =
      this.options.eventSourceFactory ?? ((url: string) => new EventSource(url) as EventSourceLike);
    let source: EventSourceLike;
    try {
      source = factory(this.url);
    } catch (error) {
      this.options.onError?.(error);
      this.scheduleReconnect();
      return;
    }
    this.source = source;
    this.lastEventAt = this.now();

    source.onopen = (): void => {
      this.attempt = 0;
      this.clearReconnectTimer();
      this.setStatus('live');
    };
    source.onerror = (error): void => {
      if (this.stopped) return;
      this.options.onError?.(error);
      this.setStatus('reconnecting');
      // Native EventSource reconnect keeps the Last-Event-ID header and the
      // daemon's replay ring. Only replace the source if that never recovers.
      this.scheduleFallbackRestart();
    };

    for (const name of SSE_EVENT_NAMES) {
      source.addEventListener(name, (message) => this.receive(name, message));
    }
  }

  private receive(name: string, message: MessageEvent<string>): void {
    this.lastEventAt = this.now();
    const event = parseSseEvent(name, message.data);
    if (!event) {
      this.options.onError?.(new Error(`discarded malformed ${name} SSE frame`));
      return;
    }
    const headerId = Number(message.lastEventId);
    const sequence = Number.isSafeInteger(headerId) && headerId > 0 ? headerId : event.seq;
    if (this.applied && sequence <= this.applied.seq && event.kind !== 'snapshot') {
      return;
    }
    this.applied = event;
    this.options.storage?.setItem(LAST_EVENT_KEY, String(sequence));
    this.attempt = 0;
    this.clearReconnectTimer();
    this.setStatus('live');
    this.options.onEvent(event);
  }

  private scheduleFallbackRestart(): void {
    if (this.stopped || this.reconnectTimer !== null) return;
    const delay = this.options.reconnectMaxMs ?? 15_000;
    this.reconnectTimer = setTimeout(() => {
      this.reconnectTimer = null;
      if (this.stopped) return;
      this.attempt += 1;
      this.source?.close();
      this.source = null;
      this.open();
    }, delay);
  }

  private clearReconnectTimer(): void {
    if (this.reconnectTimer !== null) {
      clearTimeout(this.reconnectTimer);
      this.reconnectTimer = null;
    }
  }

  private scheduleReconnect(): void {
    if (this.stopped || this.reconnectTimer !== null) return;
    const base = this.options.reconnectBaseMs ?? 500;
    const max = this.options.reconnectMaxMs ?? 15_000;
    const delay = Math.min(base * 2 ** this.attempt, max);
    this.attempt += 1;
    this.reconnectTimer = setTimeout(() => {
      this.reconnectTimer = null;
      if (!this.stopped) this.open();
    }, delay);
  }

  private startWatchdog(): void {
    if (this.watchdogTimer !== null) return;
    const staleAfter = this.options.staleAfterMs ?? 45_000;
    this.watchdogTimer = setInterval(() => {
      if (this.stopped) return;
      if (this.now() - this.lastEventAt > staleAfter) {
        this.restart();
      }
    }, Math.max(1_000, Math.floor(staleAfter / 3)));
  }

  private clearTimers(): void {
    this.clearReconnectTimer();
    if (this.watchdogTimer !== null) {
      clearInterval(this.watchdogTimer);
      this.watchdogTimer = null;
    }
  }

  private now(): number {
    return (this.options.now ?? Date.now)();
  }

  private setStatus(state: ConnectionState): void {
    this.options.onStatus?.(state);
  }
}

/** Session-backed store used by default in the browser. */
export function browserStorage(): Pick<Storage, 'getItem' | 'setItem'> | null {
  try {
    return globalThis.sessionStorage ?? null;
  } catch {
    return null;
  }
}
