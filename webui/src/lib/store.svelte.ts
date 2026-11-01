// Svelte 5 runes store. All mutations go through the pure reducers so the
// transport stays testable without a browser.

import { EventStream, browserStorage, type ConnectionState } from './events';
import { defaultFilters, filterEdges, filterNodes, isFiltered, type FilterState } from './filters';
import { computeLayout, layoutSignature, nodeSize, type NodePosition } from './layout';
import { applySseEvent, emptyRealtime, type GraphState, type RealtimeState } from './reducer';
import { RpcCallError, RpcClient, type RpcClientOptions } from './rpc';
import type {
  DaemonStatus,
  MutationDiff,
  NodeDetails,
  PolicyRecord,
  ResourceNode,
  SseEvent,
  TapHandle,
  TapEvent,
  Topology,
  ValidationResult,
} from './types';
import { resourceKey } from './types';

export type Drawer = 'inspector' | 'tap' | 'filters' | null;

export interface BootstrapOptions {
  rpc?: RpcClient;
  baseUrl?: string;
  eventSourceFactory?: (url: string) => import('./events').EventSourceLike;
}

export function describeError(error: unknown): string {
  if (error instanceof RpcCallError) {
    const detail = error.data?.details;
    return detail ? `${error.message}: ${detail}` : error.message;
  }
  if (error instanceof Error) return error.message;
  return String(error);
}

function nodeIndex(nodes: readonly ResourceNode[]): Map<string, ResourceNode> {
  return new Map(nodes.map((node) => [resourceKey(node.id), node]));
}

export class AppStore {
  status = $state<DaemonStatus | null>(null);
  realtime = $state<RealtimeState>(emptyRealtime());
  connection = $state<ConnectionState>('idle');
  loading = $state(true);
  fatalError = $state<string | null>(null);
  transportError = $state<string | null>(null);
  lastEventAt = $state<number | null>(null);

  selectedKey = $state<string | null>(null);
  details = $state<NodeDetails | null>(null);
  detailsLoading = $state(false);
  detailsError = $state<string | null>(null);
  diff = $state<MutationDiff | null>(null);
  diffLoading = $state(false);
  diffError = $state<string | null>(null);

  filters = $state<FilterState>(defaultFilters());
  positions = $state<Record<string, NodePosition>>({});

  tapHandles = $state<TapHandle[]>([]);
  tapBusy = $state(false);
  tapError = $state<string | null>(null);
  drawer = $state<Drawer>(null);
  actionMessage = $state<string | null>(null);
  validation = $state<ValidationResult | null>(null);

  readonly graph = $derived(this.realtime.graph);
  readonly nodes = $derived(this.graph.nodes);
  readonly visibleNodes = $derived(filterNodes(this.nodes, this.filters));
  readonly visibleEdges = $derived(filterEdges(this.graph.edges, this.visibleNodes));
  readonly selectedNode = $derived.by(() => {
    if (this.selectedKey === null) return null;
    return nodeIndex(this.nodes).get(this.selectedKey) ?? null;
  });
  readonly policiesForSelection = $derived.by(() => {
    if (this.selectedKey === null) return [];
    return this.realtime.policies.filter(
      (policy) => policy.resource !== null && resourceKey(policy.resource) === this.selectedKey,
    );
  });
  readonly filtered = $derived(isFiltered(this.filters));
  readonly hasGraph = $derived(this.nodes.length > 0);
  readonly canStartTap = $derived(this.selectedNode !== null);

  private rpcClient: RpcClient;
  private stream: EventStream | null = null;
  private streamFactory: ((options: ConstructorParameters<typeof EventStream>[0]) => EventStream) | null;
  private baseUrl: string;
  private lastSignature = '';
  private detailsRequest = 0;
  private diffRequest = 0;

  constructor(options: RpcClientOptions = {}) {
    this.rpcClient = new RpcClient(options);
    this.baseUrl = options.baseUrl ?? '';
    this.streamFactory = null;
  }

  get rpc(): RpcClient {
    return this.rpcClient;
  }

  get policyCount(): number {
    return this.realtime.policies.length;
  }

  async bootstrap(options: BootstrapOptions = {}): Promise<void> {
    if (options.rpc) this.rpcClient = options.rpc;
    if (options.baseUrl !== undefined) this.baseUrl = options.baseUrl;
    if (options.eventSourceFactory) {
      const factory = options.eventSourceFactory;
      this.streamFactory = (streamOptions) => {
        const rest: import('./events').EventStreamOptions = { ...streamOptions };
        delete (rest as { eventSourceFactory?: unknown }).eventSourceFactory;
        return new EventStream({ ...rest, eventSourceFactory: factory });
      };
    }
    this.loading = true;
    this.fatalError = null;
    try {
      const status = await this.rpcClient.status();
      if (status.schema_version !== this.graph.schema_version) {
        throw new Error(
          `daemon schema_version ${status.schema_version} does not match this UI (${this.graph.schema_version})`,
        );
      }
      this.status = status;
      const topology = await this.rpcClient.getTopology();
      this.ingestTopology(topology);
      this.connectEvents();
      void this.refreshTaps();
      this.loading = false;
    } catch (error) {
      this.fatalError = describeError(error);
      this.connection = 'error' as ConnectionState;
      this.loading = false;
    }
  }

  connectEvents(): void {
    this.stream?.close();
    const options = {
      baseUrl: this.baseUrl,
      storage: browserStorage(),
      onEvent: (event: SseEvent) => this.ingest(event),
      onStatus: (state: ConnectionState) => {
        this.connection = state;
        if (state === 'live') this.fatalError = null;
      },
      onError: (error: unknown) => {
        this.transportError = describeError(error);
      },
    };
    this.stream = this.streamFactory ? this.streamFactory(options) : new EventStream(options);
    this.stream.connect();
  }

  ingestTopology(topology: Topology): void {
    this.realtime = applySseEvent(this.realtime, {
      seq: 0,
      ts: new Date().toISOString(),
      kind: 'snapshot',
      data: topology,
    });
    this.lastEventAt = Date.now();
    this.syncLayout(this.realtime.graph);
  }

  ingest(event: SseEvent): void {
    this.realtime = applySseEvent(this.realtime, event);
    this.lastEventAt = Date.now();
    if (event.kind === 'daemon.error') {
      this.transportError = event.data.message;
    }
    if (event.kind === 'action.result') {
      this.actionMessage = event.data.message;
    }
    if (event.kind === 'snapshot' && this.selectedKey !== null) {
      if (!nodeIndex(this.realtime.graph.nodes).has(this.selectedKey)) {
        this.clearSelection();
      }
    }
    this.syncLayout(this.realtime.graph);
  }

  /** Recomputes dagre only when the node/edge identity set changed. */
  syncLayout(graph: GraphState = this.graph): void {
    const signature = layoutSignature(graph.nodes, graph.edges);
    if (signature === this.lastSignature && Object.keys(this.positions).length > 0) {
      return;
    }
    this.lastSignature = signature;
    this.positions = computeLayout(graph.nodes, graph.edges);
  }

  selectNode(node: ResourceNode): void {
    const key = resourceKey(node.id);
    if (this.selectedKey === key) {
      this.clearSelection();
      return;
    }
    this.selectedKey = key;
    this.drawer = 'inspector';
    void this.loadDetails(node);
  }

  clearSelection(): void {
    this.selectedKey = null;
    this.details = null;
    this.detailsError = null;
    this.diff = null;
    this.diffError = null;
    this.detailsRequest += 1;
    this.diffRequest += 1;
    if (this.drawer === 'inspector') this.drawer = null;
  }

  async loadDetails(node: ResourceNode): Promise<void> {
    const request = ++this.detailsRequest;
    this.detailsLoading = true;
    this.detailsError = null;
    this.details = null;
    this.diff = null;
    try {
      const details = await this.rpcClient.getNodeDetails(node.id, true);
      if (request !== this.detailsRequest) return;
      this.details = details;
    } catch (error) {
      if (request !== this.detailsRequest) return;
      this.detailsError = describeError(error);
    } finally {
      if (request === this.detailsRequest) this.detailsLoading = false;
    }
  }

  async loadDiff(): Promise<void> {
    const node = this.selectedNode;
    if (!node) return;
    const request = ++this.diffRequest;
    this.diffLoading = true;
    this.diffError = null;
    try {
      const diff = await this.rpcClient.getDiff(node.id, 'three_way');
      if (request !== this.diffRequest) return;
      this.diff = diff;
    } catch (error) {
      if (request !== this.diffRequest) return;
      this.diffError = describeError(error);
    } finally {
      if (request === this.diffRequest) this.diffLoading = false;
    }
  }

  async refresh(): Promise<void> {
    try {
      await this.rpcClient.refresh();
      const topology = await this.rpcClient.getTopology();
      this.ingestTopology(topology);
    } catch (error) {
      this.transportError = describeError(error);
    }
  }

  async refreshTaps(): Promise<void> {
    try {
      this.tapHandles = await this.rpcClient.tapList();
    } catch (error) {
      if (error instanceof RpcCallError && (error.code === -32601 || error.code === -32006)) {
        this.tapHandles = [];
        return;
      }
      this.tapError = describeError(error);
    }
  }

  async startTap(kind: 'broker' | 'trigger', namespace: string, name: string, typeFilter = ''): Promise<void> {
    this.tapBusy = true;
    this.tapError = null;
    try {
      const attributes: Record<string, string> = {};
      if (typeFilter.trim() !== '') attributes['type'] = typeFilter.trim();
      const handle = await this.rpcClient.tapStart({
        target: { kind, namespace, name },
        filter: { attributes },
        max_events: 100,
        ttl_secs: 900,
      });
      this.tapHandles = [...this.tapHandles.filter((tap) => tap.id !== handle.id), handle];
      this.drawer = 'tap';
    } catch (error) {
      this.tapError = describeError(error);
    } finally {
      this.tapBusy = false;
    }
  }

  async stopTap(id: string): Promise<void> {
    this.tapBusy = true;
    this.tapError = null;
    try {
      await this.rpcClient.tapStop(id);
      this.tapHandles = this.tapHandles.filter((tap) => tap.id !== id);
    } catch (error) {
      this.tapError = describeError(error);
    } finally {
      this.tapBusy = false;
    }
  }

  setFilter<K extends keyof FilterState>(key: K, value: FilterState[K]): void {
    this.filters[key] = value;
  }

  toggleCategory(category: FilterState['hiddenCategories'][number]): void {
    const hidden = this.filters.hiddenCategories;
    this.filters.hiddenCategories = hidden.includes(category)
      ? hidden.filter((candidate) => candidate !== category)
      : [...hidden, category];
  }

  resetFilters(): void {
    this.filters = defaultFilters();
  }

  setDrawer(drawer: Drawer): void {
    this.drawer = drawer;
  }

  dismissTransportError(): void {
    this.transportError = null;
  }

  dismissAction(): void {
    this.actionMessage = null;
  }

  setValidation(result: ValidationResult | null): void {
    this.validation = result;
  }

  tapEvents(): TapEvent[] {
    return this.realtime.tapEvents;
  }

  policies(): PolicyRecord[] {
    return this.realtime.policies;
  }

  sizeOf(node: ResourceNode): { width: number; height: number } {
    return nodeSize(node.category);
  }

  close(): void {
    this.stream?.close();
    this.stream = null;
  }
}

export const store = new AppStore();
