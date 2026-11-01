//! Filterable resource tree grouped by namespace and kind.

use std::collections::BTreeMap;

use knect_proto::graph::{Health, NodeCategory};
use knect_proto::{ResourceId, ResourceNode, Topology};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Paragraph};

use crate::app::state::AppState;
use crate::ui::theme;

/// One rendered tree row. Group headers have no resource.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeRow {
    /// Display text.
    pub label: String,
    /// Resource for selectable rows.
    pub resource: Option<ResourceId>,
    /// Indentation depth.
    pub depth: u8,
    /// Node health for selectable rows.
    pub health: Option<Health>,
    /// Short scale/health badge.
    pub badge: Option<String>,
}

impl TreeRow {
    /// True when arrow-key navigation can land on this row.
    #[must_use]
    pub fn selectable(&self) -> bool {
        self.resource.is_some()
    }
}

/// Builds tree rows for a topology, applying a case-insensitive filter.
#[must_use]
pub fn build_rows(topology: Option<&Topology>, filter: &str) -> Vec<TreeRow> {
    let Some(topology) = topology else {
        return Vec::new();
    };
    let needle = filter.trim().to_lowercase();
    let mut namespaces: BTreeMap<Option<String>, BTreeMap<NodeCategory, Vec<&ResourceNode>>> =
        BTreeMap::new();
    for node in &topology.nodes {
        if !matches_filter(node, &needle) {
            continue;
        }
        namespaces
            .entry(node.namespace.clone())
            .or_default()
            .entry(node.category)
            .or_default()
            .push(node);
    }
    let mut rows = Vec::new();
    for (namespace, categories) in namespaces {
        let namespace_label = namespace
            .as_deref()
            .filter(|value| !value.is_empty())
            .unwrap_or("cluster-scoped");
        rows.push(TreeRow {
            label: format!("▾ {namespace_label}"),
            resource: None,
            depth: 0,
            health: None,
            badge: None,
        });
        for (category, mut nodes) in categories {
            nodes.sort_by(|left, right| left.name.cmp(&right.name));
            rows.push(TreeRow {
                label: format!("▾ {} {}", category_title(category), nodes.len()),
                resource: None,
                depth: 1,
                health: None,
                badge: None,
            });
            for node in nodes {
                rows.push(TreeRow {
                    label: node.name.clone(),
                    resource: Some(node.id.clone()),
                    depth: 2,
                    health: Some(node.health),
                    badge: scale_badge(node),
                });
            }
        }
    }
    rows
}

fn matches_filter(node: &ResourceNode, needle: &str) -> bool {
    if needle.is_empty() {
        return true;
    }
    let category = category_title(node.category);
    node.name.to_lowercase().contains(needle)
        || node
            .namespace
            .as_deref()
            .is_some_and(|namespace| namespace.to_lowercase().contains(needle))
        || category.to_lowercase().contains(needle)
        || node.summary.to_lowercase().contains(needle)
}

/// Computes a compact scale or health badge from node metadata.
#[must_use]
pub fn scale_badge(node: &ResourceNode) -> Option<String> {
    let badge_labels: Vec<String> = node
        .badges
        .iter()
        .map(|badge| badge.label.to_lowercase())
        .collect();
    let canary = badge_labels.iter().any(|label| label.contains("canary"))
        || node
            .labels
            .values()
            .any(|value| value.eq_ignore_ascii_case("canary"));
    if canary {
        return Some("Canary".to_owned());
    }
    if matches!(node.health, Health::Degraded | Health::Blocked) {
        return Some("Degraded".to_owned());
    }
    let scale_to_zero = badge_labels
        .iter()
        .any(|label| label.contains("scale-to-zero"))
        || node
            .annotations
            .get("autoscaling.knative.dev/min-scale")
            .is_some_and(|value| value == "0");
    if scale_to_zero {
        return Some("0K".to_owned());
    }
    match node.category {
        NodeCategory::KService
        | NodeCategory::Revision
        | NodeCategory::PodAutoscaler
        | NodeCategory::Deployment
        | NodeCategory::ReplicaSet
        | NodeCategory::Pod => Some("1..N".to_owned()),
        _ => None,
    }
}

/// Group title for a node category.
#[must_use]
pub fn category_title(category: NodeCategory) -> &'static str {
    match category {
        NodeCategory::KService => "Services",
        NodeCategory::Route => "Routes",
        NodeCategory::Configuration => "Configurations",
        NodeCategory::Revision => "Revisions",
        NodeCategory::PodAutoscaler => "PodAutoscalers",
        NodeCategory::Broker => "Brokers",
        NodeCategory::Trigger => "Triggers",
        NodeCategory::Source => "Sources",
        NodeCategory::SinkBinding => "SinkBindings",
        NodeCategory::Subscription => "Subscriptions",
        NodeCategory::Channel => "Channels",
        NodeCategory::Deployment => "Deployments",
        NodeCategory::ReplicaSet => "ReplicaSets",
        NodeCategory::Pod => "Pods",
        NodeCategory::Service => "ClusterIP Services",
        NodeCategory::Endpoints => "Endpoints",
        NodeCategory::BackingService => "Backing Services",
        NodeCategory::Topic => "Topics",
        NodeCategory::StorageClass => "StorageClasses",
        NodeCategory::PersistentVolume => "PersistentVolumes",
        NodeCategory::PersistentVolumeClaim => "PVCs",
        NodeCategory::CephPool => "CephPools",
        NodeCategory::Policy => "Policies",
        NodeCategory::Canary => "Canaries",
        NodeCategory::VirtualService => "VirtualServices",
        NodeCategory::DestinationRule => "DestinationRules",
        NodeCategory::PeerAuthentication => "PeerAuthentications",
        NodeCategory::AuthorizationPolicy => "AuthorizationPolicies",
        NodeCategory::ConfigMap => "ConfigMaps",
        NodeCategory::Secret => "Secrets",
        NodeCategory::Namespace => "Namespaces",
        NodeCategory::Unknown => "Unknown",
    }
}

/// Indexes of selectable rows.
#[must_use]
pub fn selectable_indices(rows: &[TreeRow]) -> Vec<usize> {
    rows.iter()
        .enumerate()
        .filter_map(|(index, row)| row.selectable().then_some(index))
        .collect()
}

/// Renders the tree pane.
pub fn render(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    let theme = theme::knect_dark();
    let focused = state.active_pane == crate::app::state::Pane::Tree;
    let filter = if state.filter.is_empty() {
        String::new()
    } else {
        format!("  /{}", state.filter)
    };
    let title = Line::from(vec![
        Span::styled(
            "Resources",
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(filter, Style::default().fg(theme.muted)),
    ]);
    let block = Block::bordered()
        .title(title)
        .border_style(theme.pane_border(focused));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if state.tree_rows.is_empty() {
        let message = if state.topology.is_none() {
            "waiting for topology…"
        } else if state.filter.is_empty() {
            "no Knative resources discovered"
        } else {
            "no resources match the filter"
        };
        frame.render_widget(
            Paragraph::new(message).style(Style::default().fg(theme.muted)),
            inner,
        );
        return;
    }

    let height = usize::from(inner.height);
    let selected = state
        .selected_row
        .min(state.tree_rows.len().saturating_sub(1));
    let mut offset = state.tree_offset;
    if height > 0 {
        if selected < offset {
            offset = selected;
        } else if selected >= offset + height {
            offset = selected + 1 - height;
        }
    }
    let visible = state.tree_rows.iter().enumerate().skip(offset).take(height);
    let mut lines = Vec::new();
    for (index, row) in visible {
        let mut style = if row.selectable() {
            Style::default().fg(theme.text)
        } else {
            theme.group()
        };
        if index == selected && row.selectable() {
            style = theme.selection();
        }
        let mut spans = vec![Span::styled(
            format!("{}{}", " ".repeat(usize::from(row.depth)), row.label),
            style,
        )];
        if let Some(health) = row.health {
            spans.push(Span::styled(
                format!(" {}", health_glyph(health)),
                Style::default().fg(theme.health(health)),
            ));
        }
        if let Some(badge) = &row.badge {
            spans.push(Span::styled(
                format!(" [{badge}]"),
                Style::default().fg(badge_color(badge, &theme)),
            ));
        }
        lines.push(Line::from(spans));
    }
    frame.render_widget(Paragraph::new(Text::from(lines)), inner);
}

fn health_glyph(health: Health) -> &'static str {
    match health {
        Health::Healthy => "●",
        Health::Degraded => "▲",
        Health::Progressing => "◐",
        Health::Blocked => "✖",
        Health::Unknown => "○",
    }
}

fn badge_color(badge: &str, theme: &theme::Theme) -> ratatui::style::Color {
    match badge {
        "Degraded" => theme.blocked,
        "Canary" => theme.degraded,
        "0K" => theme.eventing,
        _ => theme.muted,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use knect_proto::graph::{Badge, Severity, TopologyStats};

    fn node(namespace: Option<&str>, category: NodeCategory, name: &str) -> ResourceNode {
        ResourceNode {
            id: ResourceId::new(
                "example.test/v1",
                "Thing",
                namespace.map(str::to_owned),
                name,
            ),
            category,
            name: name.to_owned(),
            namespace: namespace.map(str::to_owned),
            health: Health::Healthy,
            summary: format!("summary for {name}"),
            labels: BTreeMap::new(),
            annotations: BTreeMap::new(),
            badges: Vec::new(),
            uid: None,
            first_seen: Utc::now(),
            last_seen: Utc::now(),
        }
    }

    fn topology(nodes: Vec<ResourceNode>) -> Topology {
        Topology {
            schema_version: 1,
            context: "test".to_owned(),
            generation: 1,
            stats: TopologyStats::default(),
            nodes,
            edges: Vec::new(),
            warnings: Vec::new(),
        }
    }

    #[test]
    fn rows_group_by_namespace_then_kind() {
        let rows = build_rows(
            Some(&topology(vec![
                node(Some("b"), NodeCategory::Revision, "rev-b"),
                node(Some("a"), NodeCategory::KService, "svc-a"),
                node(Some("a"), NodeCategory::Revision, "rev-a"),
            ])),
            "",
        );
        let labels: Vec<&str> = rows.iter().map(|row| row.label.as_str()).collect();
        assert_eq!(labels[0], "▾ a");
        assert_eq!(labels[1], "▾ Services 1");
        assert_eq!(labels[3], "▾ Revisions 1");
        assert_eq!(
            rows[2].resource.as_ref().map(|id| id.name.as_str()),
            Some("svc-a")
        );
        assert_eq!(rows[2].depth, 2);
        assert!(rows[2].selectable());
        assert!(!rows[0].selectable());
    }

    #[test]
    fn filter_matches_name_kind_namespace_and_summary() {
        let topology = topology(vec![
            node(Some("orders"), NodeCategory::KService, "checkout"),
            node(Some("default"), NodeCategory::Broker, "events"),
        ]);
        assert_eq!(build_rows(Some(&topology), "check").len(), 3);
        assert_eq!(build_rows(Some(&topology), "broker").len(), 3);
        assert_eq!(build_rows(Some(&topology), "default").len(), 3);
        assert_eq!(build_rows(Some(&topology), "events").len(), 3);
        assert!(build_rows(Some(&topology), "missing").is_empty());
    }

    #[test]
    fn scale_badges_cover_zero_canary_and_degraded() {
        let mut zero = node(Some("a"), NodeCategory::KService, "zero");
        zero.badges.push(Badge {
            label: "scale-to-zero".to_owned(),
            severity: Severity::Info,
            tooltip: None,
        });
        assert_eq!(scale_badge(&zero).as_deref(), Some("0K"));

        let mut canary = node(Some("a"), NodeCategory::Revision, "canary");
        canary
            .labels
            .insert("traffic".to_owned(), "canary".to_owned());
        assert_eq!(scale_badge(&canary).as_deref(), Some("Canary"));

        let mut degraded = node(Some("a"), NodeCategory::Revision, "rev");
        degraded.health = Health::Degraded;
        assert_eq!(scale_badge(&degraded).as_deref(), Some("Degraded"));

        let plain = node(Some("a"), NodeCategory::Revision, "plain");
        assert_eq!(scale_badge(&plain).as_deref(), Some("1..N"));
        let broker = node(Some("a"), NodeCategory::Broker, "b");
        assert_eq!(scale_badge(&broker), None);
    }

    #[test]
    fn selectable_indices_skip_headers() {
        let rows = build_rows(
            Some(&topology(vec![node(
                Some("a"),
                NodeCategory::KService,
                "svc",
            )])),
            "",
        );
        assert_eq!(selectable_indices(&rows), vec![2]);
    }

    #[test]
    fn cluster_scoped_resources_get_a_header() {
        let rows = build_rows(
            Some(&topology(vec![node(None, NodeCategory::Namespace, "prod")])),
            "",
        );
        assert_eq!(rows[0].label, "▾ cluster-scoped");
    }
}
