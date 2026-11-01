// Presentation metadata for the frozen protocol enums. Components read from
// here so a category colour or label changes in one place.

import type { EdgeKind, Health, NodeCategory, Severity } from './types';

export type CategoryGroup =
  | 'kservice'
  | 'revision'
  | 'broker'
  | 'trigger'
  | 'backing'
  | 'storage'
  | 'policy'
  | 'generic';

export interface CategoryMeta {
  label: string;
  group: CategoryGroup;
  /** 24x24 stroke path data. Decorative; the label carries the meaning. */
  icon: string;
}

const CATEGORY: Record<NodeCategory, CategoryMeta> = {
  k_service: { label: 'KService', group: 'kservice', icon: 'M4 8.5 12 4l8 4.5v7L12 20l-8-4.5z M4 8.5l8 4.5 8-4.5 M12 13v7' },
  route: { label: 'Route', group: 'generic', icon: 'M6 19a3 3 0 1 1 2.8-4h6.4a3 3 0 1 0 0-6H9' },
  configuration: { label: 'Configuration', group: 'generic', icon: 'M4 7h10 M18 7h2 M4 17h2 M10 17h10 M14 4v6 M8 14v6' },
  revision: { label: 'Revision', group: 'revision', icon: 'M12 3 3 7.5 12 12l9-4.5z M3 12l9 4.5 9-4.5 M3 16.5 12 21l9-4.5' },
  pod_autoscaler: { label: 'Autoscaler', group: 'generic', icon: 'M4 18V9 M10 18V5 M16 18v-6 M22 18V8' },
  broker: { label: 'Broker', group: 'broker', icon: 'M12 12a2 2 0 1 0 0-4 2 2 0 0 0 0 4z M7.4 7.4a6 6 0 0 0 0 8.5 M16.6 7.4a6 6 0 0 1 0 8.5 M4 4a11 11 0 0 0 0 16 M20 4a11 11 0 0 1 0 16' },
  trigger: { label: 'Trigger', group: 'trigger', icon: 'M13 2 4 14h7l-1 8 9-12h-7z' },
  source: { label: 'Source', group: 'generic', icon: 'M5 12a7 7 0 0 1 7-7 M5 12a7 7 0 0 0 7 7 M5 12h.01 M9 12a3 3 0 0 1 3-3' },
  sink_binding: { label: 'SinkBinding', group: 'generic', icon: 'M8 7h8 M8 17h8 M6 7v10 M18 7v10' },
  subscription: { label: 'Subscription', group: 'generic', icon: 'M4 6h16 M4 12h16 M4 18h10' },
  channel: { label: 'Channel', group: 'generic', icon: 'M4 7h16 M4 12h16 M4 17h16' },
  deployment: { label: 'Deployment', group: 'generic', icon: 'M4 7 12 3l8 4v10l-8 4-8-4z M4 7l8 4 8-4 M12 11v10' },
  replica_set: { label: 'ReplicaSet', group: 'generic', icon: 'M8 4h8v16H8z M5 7v10 M19 7v10' },
  pod: { label: 'Pod', group: 'generic', icon: 'M12 3 3 8v8l9 5 9-5V8z M3 8l9 5 9-5 M12 13v8' },
  service: { label: 'Service', group: 'generic', icon: 'M12 4a3 3 0 1 0 0 6 3 3 0 0 0 0-6 M5 20a3 3 0 1 0 0-6 3 3 0 0 0 0 6 M19 20a3 3 0 1 0 0-6 3 3 0 0 0 0 6 M10.5 9.5 6.5 14.5 M13.5 9.5l4 5' },
  endpoints: { label: 'Endpoints', group: 'generic', icon: 'M4 12h16 M12 4v16 M7 7l10 10 M17 7 7 17' },
  backing_service: { label: 'BackingService', group: 'backing', icon: 'M4 6c0-1.7 3.6-3 8-3s8 1.3 8 3-3.6 3-8 3-8-1.3-8-3z M4 6v6c0 1.7 3.6 3 8 3s8-1.3 8-3V6 M4 12v6c0 1.7 3.6 3 8 3s8-1.3 8-3v-6' },
  topic: { label: 'Topic', group: 'backing', icon: 'M4 6h16 M4 12h16 M4 18h16' },
  storage_class: { label: 'StorageClass', group: 'storage', icon: 'M4 7h16v12H4z M4 11h16 M8 15h4' },
  persistent_volume: { label: 'PersistentVolume', group: 'storage', icon: 'M4 6h16v12H4z M4 10h16' },
  persistent_volume_claim: { label: 'PVC', group: 'storage', icon: 'M6 3h9l4 4v14H6z M15 3v4h4' },
  ceph_pool: { label: 'CephPool', group: 'storage', icon: 'M12 3 3 8v8l9 5 9-5V8z M12 8v8' },
  policy: { label: 'Policy', group: 'policy', icon: 'M12 3 4 6v6c0 5 3.4 8 8 9 4.6-1 8-4 8-9V6z M9 12l2 2 4-4' },
  canary: { label: 'Canary', group: 'generic', icon: 'M9 3h6v4l3 5v6l-3 3H9l-3-3v-6l3-5z M9 13h6' },
  virtual_service: { label: 'VirtualService', group: 'generic', icon: 'M4 18c4-10 12-10 16 0 M8 18c2-5 6-5 8 0' },
  destination_rule: { label: 'DestinationRule', group: 'generic', icon: 'M4 6h16 M4 12h10 M4 18h6' },
  peer_authentication: { label: 'PeerAuthentication', group: 'policy', icon: 'M12 3 4 6v6c0 5 3.4 8 8 9 4.6-1 8-4 8-9V6z' },
  authorization_policy: { label: 'AuthorizationPolicy', group: 'policy', icon: 'M12 3 4 6v6c0 5 3.4 8 8 9 4.6-1 8-4 8-9V6z M8 12h8' },
  config_map: { label: 'ConfigMap', group: 'generic', icon: 'M5 4h10l4 4v12H5z M15 4v4h4 M8 12h8 M8 16h6' },
  secret: { label: 'Secret', group: 'generic', icon: 'M6 10V7a6 6 0 0 1 12 0v3 M5 10h14v11H5z M12 14v3' },
  namespace: { label: 'Namespace', group: 'generic', icon: 'M4 5h16v14H4z M8 5v14 M8 10h12' },
  unknown: { label: 'Unknown', group: 'generic', icon: 'M12 3a9 9 0 1 0 0 18 9 9 0 0 0 0-18 M12 8v1 M12 12v4' },
};

export function categoryMeta(category: NodeCategory): CategoryMeta {
  return CATEGORY[category] ?? CATEGORY.unknown;
}

export function categoryLabel(category: NodeCategory): string {
  return categoryMeta(category).label;
}

export function categoryGroup(category: NodeCategory): CategoryGroup {
  return categoryMeta(category).group;
}

export function categoryColor(category: NodeCategory): string {
  return `var(--cat-${categoryMeta(category).group})`;
}

export function groupColor(group: CategoryGroup): string {
  return `var(--cat-${group})`;
}

export type Tone = 'ok' | 'warn' | 'danger' | 'progress' | 'neutral';

export interface HealthMeta {
  label: string;
  tone: Tone;
}

const HEALTH: Record<Health, HealthMeta> = {
  healthy: { label: 'Healthy', tone: 'ok' },
  degraded: { label: 'Degraded', tone: 'warn' },
  progressing: { label: 'Progressing', tone: 'progress' },
  blocked: { label: 'Blocked', tone: 'danger' },
  unknown: { label: 'Unknown', tone: 'neutral' },
};

export function healthMeta(health: Health): HealthMeta {
  return HEALTH[health] ?? HEALTH.unknown;
}

const SEVERITY: Record<Severity, Tone> = {
  critical: 'danger',
  high: 'danger',
  medium: 'warn',
  low: 'progress',
  info: 'neutral',
  unknown: 'neutral',
};

export function severityTone(severity: Severity): Tone {
  return SEVERITY[severity] ?? 'neutral';
}

export function severityLabel(severity: Severity): string {
  return severity.charAt(0).toUpperCase() + severity.slice(1);
}

export function edgeLabel(kind: EdgeKind): string {
  const labels: Partial<Record<EdgeKind, string>> = {
    routes_to: 'routes to',
    owns: 'owns',
    scales: 'scales',
    runs: 'runs',
    subscribes: 'subscribes',
    delivers_to: 'delivers',
    dead_letter_sink: 'dead letter',
    uses_secret: 'uses secret',
    uses_config_map: 'uses config',
    connects_to: 'connects',
    backed_by: 'backed by',
    mounts: 'mounts',
    provisioned_by: 'provisioned by',
    protected_by: 'protects',
    mutated_by: 'mutates',
    manages: 'manages',
    references: 'references',
    targets: 'targets',
    published_by: 'published by',
    consumes_from: 'consumes',
    selects: 'selects',
    unknown: 'link',
  };
  return labels[kind] ?? kind.replaceAll('_', ' ');
}

export const TRAFFIC_EDGE_KINDS: ReadonlySet<EdgeKind> = new Set([
  'routes_to',
  'delivers_to',
  'subscribes',
  'published_by',
  'consumes_from',
]);

/** True when an edge represents live request or CloudEvent flow. */
export function edgeHasTraffic(kind: EdgeKind, metadata: Record<string, unknown>): boolean {
  if (!TRAFFIC_EDGE_KINDS.has(kind)) return false;
  const percent = metadata['traffic_percent'];
  if (typeof percent === 'number') return percent > 0;
  const taps = metadata['tap_count'];
  if (typeof taps === 'number') return taps > 0;
  return metadata['traffic'] === true || metadata['live'] === true;
}

export function formatNumber(value: number): string {
  if (!Number.isFinite(value)) return '—';
  if (Math.abs(value) >= 1_000_000) return `${(value / 1_000_000).toFixed(1)}M`;
  if (Math.abs(value) >= 1_000) return `${(value / 1_000).toFixed(1)}k`;
  return Number.isInteger(value) ? String(value) : value.toFixed(1);
}

export function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  const units = ['KiB', 'MiB', 'GiB', 'TiB'];
  let value = bytes / 1024;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  return `${value.toFixed(1)} ${units[unit] ?? 'B'}`;
}

export function formatAge(timestamp: string, now = Date.now()): string {
  const then = Date.parse(timestamp);
  if (Number.isNaN(then)) return '—';
  const seconds = Math.max(0, Math.floor((now - then) / 1000));
  if (seconds < 60) return `${seconds}s`;
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes}m`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours}h`;
  return `${Math.floor(hours / 24)}d`;
}

/** Colour used for a mutation owner chip; stable per manager family. */
export function ownerTone(owner: string | null): string {
  if (!owner) return 'var(--cat-generic)';
  const name = owner.toLowerCase();
  if (name.includes('kubectl')) return 'var(--cat-kservice)';
  if (name.includes('knative')) return 'var(--cat-revision)';
  if (name.includes('istio')) return 'var(--cat-trigger)';
  if (name.includes('kyverno')) return 'var(--cat-policy)';
  if (name.includes('flagger')) return 'var(--cat-broker)';
  if (name.includes('helm')) return 'var(--cat-backing)';
  return 'var(--cat-storage)';
}
