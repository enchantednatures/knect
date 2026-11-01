import { render, screen } from '@testing-library/svelte';
import { describe, expect, it } from 'vitest';
import NodeShell from '../lib/nodes/NodeShell.svelte';
import { buildFlowEdges, buildFlowNodes, nodeTypeFor } from '../lib/flow';
import { revision, topology } from './fixtures';

describe('node rendering', () => {
  it('renders kind, name, health text, badges, and policy counts', () => {
    render(NodeShell, {
      props: {
        node: revision,
        showHandles: false,
        policyCount: 3,
        failingPolicies: 1,
      },
    });

    expect(screen.getByText('Revision')).toBeInTheDocument();
    expect(screen.getByText('hello-00001')).toBeInTheDocument();
    expect(screen.getByText('Healthy')).toBeInTheDocument();
    expect(screen.getByText('ready')).toBeInTheDocument();
    expect(screen.getByTestId('policy-badge')).toHaveTextContent('1 failing');
    expect(screen.queryByText('0 failing')).not.toBeInTheDocument();
    expect(screen.getByTestId('node-shell')).toHaveAttribute('data-health', 'healthy');
  });

  it('renders a selected ring hook and cluster-scoped labels', () => {
    const { container } = render(NodeShell, {
      props: {
        node: { ...revision, namespace: null, summary: '', badges: [] },
        showHandles: false,
        selected: true,
      },
    });
    expect(screen.getByTestId('node-shell')).toHaveClass('node-shell--selected');
    expect(screen.getByText(/cluster/)).toBeInTheDocument();
    expect(screen.getByText('No summary reported')).toBeInTheDocument();
    expect(container.querySelectorAll('.node-rail')).toHaveLength(1);
  });
});

describe('flow mapping', () => {
  it('maps protocol categories to custom node types', () => {
    expect(nodeTypeFor('k_service')).toBe('kservice');
    expect(nodeTypeFor('revision')).toBe('revision');
    expect(nodeTypeFor('broker')).toBe('broker');
    expect(nodeTypeFor('trigger')).toBe('trigger');
    expect(nodeTypeFor('backing_service')).toBe('backing_service');
    expect(nodeTypeFor('persistent_volume_claim')).toBe('storage');
    expect(nodeTypeFor('policy')).toBe('policy');
    expect(nodeTypeFor('deployment')).toBe('resource');
  });

  it('builds accessible flow nodes with stable identities', () => {
    const nodes = buildFlowNodes(topology.nodes, {}, null, new Map());
    const revisionNode = nodes.find((node) => node.id.includes('Revision'));
    expect(revisionNode).toBeDefined();
    expect(revisionNode?.ariaLabel).toContain('Revision hello-00001 in default, Healthy');
    expect(revisionNode?.type).toBe('revision');
    expect(nodes).toHaveLength(topology.nodes.length);
  });

  it('animates traffic edges and labels their percentage', () => {
    const edges = buildFlowEdges(topology.edges);
    const traffic = edges.find((edge) => edge.id === 'edge-trigger-broker');
    const staticEdge = edges.find((edge) => edge.id === 'edge-kservice-revision');
    expect(traffic?.animated).toBe(true);
    expect(traffic?.label).toBe('100%');
    expect(staticEdge?.animated).toBe(false);
  });
});
