//! Deterministic layered DAG layout and renderer for the topology pane.

use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet, BinaryHeap};

use knect_proto::graph::{EdgeKind, Health, NodeCategory, Topology};
use knect_proto::{Edge, ResourceId, ResourceNode};
use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Paragraph, Widget};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::app::state::AppState;
use crate::ui::{theme, tree};

const NODE_HEIGHT: i32 = 3;
const NODE_GAP_Y: i32 = 1;
const NODE_GAP_X: i32 = 4;
const MIN_NODE_WIDTH: i32 = 16;
const MAX_NODE_WIDTH: i32 = 24;

/// One positioned node box.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayoutNode {
    /// Resource identity.
    pub id: ResourceId,
    /// Layer index (column).
    pub layer: usize,
    /// Left edge in layout coordinates.
    pub x: i32,
    /// Top edge in layout coordinates.
    pub y: i32,
    /// Box width.
    pub width: i32,
    /// Box height.
    pub height: i32,
    /// Label rendered inside the box.
    pub label: String,
    /// Node category.
    pub category: NodeCategory,
    /// Node health.
    pub health: Health,
    /// Short badges rendered after the label.
    pub badges: Vec<String>,
}

/// One routed edge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayoutEdge {
    /// Source resource.
    pub from: ResourceId,
    /// Target resource.
    pub to: ResourceId,
    /// Semantic relationship.
    pub kind: EdgeKind,
    /// Optional label.
    pub label: Option<String>,
    /// Edge health.
    pub health: Health,
    /// Orthogonal polyline in layout coordinates.
    pub points: Vec<(i32, i32)>,
}

/// Complete layout for one topology generation.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GraphLayout {
    /// Positioned nodes, ordered deterministically.
    pub nodes: Vec<LayoutNode>,
    /// Routed edges, ordered deterministically.
    pub edges: Vec<LayoutEdge>,
    /// Canvas width in cells.
    pub width: i32,
    /// Canvas height in cells.
    pub height: i32,
    /// Number of layers.
    pub layers: usize,
}

/// Builds a deterministic layered layout from topology edges.
#[must_use]
pub fn layout(topology: &Topology) -> GraphLayout {
    if topology.nodes.is_empty() {
        return GraphLayout::default();
    }
    let mut nodes: Vec<&ResourceNode> = topology.nodes.iter().collect();
    nodes.sort_by_key(|node| identity_key(&node.id));
    let index: BTreeMap<ResourceId, usize> = nodes
        .iter()
        .enumerate()
        .map(|(position, node)| (node.id.clone(), position))
        .collect();

    let count = nodes.len();
    let mut outgoing: Vec<Vec<usize>> = vec![Vec::new(); count];
    let mut incoming: Vec<Vec<usize>> = vec![Vec::new(); count];
    let mut seen: BTreeSet<(usize, usize)> = BTreeSet::new();
    for edge in &topology.edges {
        let (Some(&from), Some(&to)) = (index.get(&edge.from), index.get(&edge.to)) else {
            continue;
        };
        if from == to || !seen.insert((from, to)) {
            continue;
        }
        outgoing[from].push(to);
        incoming[to].push(from);
    }
    for list in &mut outgoing {
        list.sort_unstable();
    }
    for list in &mut incoming {
        list.sort_unstable();
    }

    let layers = assign_layers(count, &outgoing, &incoming);
    let mut by_layer: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for (node, layer) in layers.iter().enumerate() {
        by_layer.entry(*layer).or_default().push(node);
    }
    for list in by_layer.values_mut() {
        list.sort_unstable();
    }

    let mut layer_widths: BTreeMap<usize, i32> = BTreeMap::new();
    for (layer, members) in &by_layer {
        let width = members
            .iter()
            .map(|node| node_width(nodes[*node]))
            .max()
            .unwrap_or(MIN_NODE_WIDTH);
        layer_widths.insert(*layer, width);
    }

    let mut positioned: BTreeMap<usize, LayoutNode> = BTreeMap::new();
    let mut x = 0_i32;
    let mut max_bottom = 0_i32;
    let mut height = 0_i32;
    for (layer, members) in &by_layer {
        let width = layer_widths.get(layer).copied().unwrap_or(MIN_NODE_WIDTH);
        for (row, node_index) in members.iter().enumerate() {
            let node = nodes[*node_index];
            let y = row as i32 * (NODE_HEIGHT + NODE_GAP_Y);
            let badges = badges_for(node);
            positioned.insert(
                *node_index,
                LayoutNode {
                    id: node.id.clone(),
                    layer: *layer,
                    x,
                    y,
                    width,
                    height: NODE_HEIGHT,
                    label: format!("{} {}", short_category(node.category), node.name),
                    category: node.category,
                    health: node.health,
                    badges,
                },
            );
            max_bottom = max_bottom.max(y + NODE_HEIGHT);
        }
        height = height.max(max_bottom);
        x = x.saturating_add(width).saturating_add(NODE_GAP_X);
    }
    let width = x.saturating_sub(NODE_GAP_X);

    let mut layout_nodes: Vec<LayoutNode> = positioned.into_values().collect();
    layout_nodes.sort_by(|left, right| {
        (left.x, left.y)
            .cmp(&(right.x, right.y))
            .then_with(|| identity_key(&left.id).cmp(&identity_key(&right.id)))
    });

    let by_id: BTreeMap<&ResourceId, &LayoutNode> =
        layout_nodes.iter().map(|node| (&node.id, node)).collect();
    let mut layout_edges = Vec::new();
    for edge in &topology.edges {
        let (Some(from), Some(to)) = (by_id.get(&edge.from), by_id.get(&edge.to)) else {
            continue;
        };
        if from.id == to.id {
            continue;
        }
        layout_edges.push(LayoutEdge {
            from: from.id.clone(),
            to: to.id.clone(),
            kind: edge.kind,
            label: edge.label.clone(),
            health: edge.health,
            points: route(from, to),
        });
    }
    layout_edges.sort_by(|left, right| {
        identity_key(&left.from)
            .cmp(&identity_key(&right.from))
            .then_with(|| identity_key(&left.to).cmp(&identity_key(&right.to)))
    });

    GraphLayout {
        nodes: layout_nodes,
        edges: layout_edges,
        width,
        height,
        layers: by_layer.len(),
    }
}

fn assign_layers(count: usize, outgoing: &[Vec<usize>], incoming: &[Vec<usize>]) -> Vec<usize> {
    let mut indegree: Vec<usize> = incoming.iter().map(Vec::len).collect();
    let mut layer = vec![0_usize; count];
    let mut done = vec![false; count];
    let mut queue: BinaryHeap<Reverse<usize>> = (0..count)
        .filter(|node| indegree[*node] == 0)
        .map(Reverse)
        .collect();
    while let Some(Reverse(node)) = queue.pop() {
        if done[node] {
            continue;
        }
        done[node] = true;
        for &target in &outgoing[node] {
            layer[target] = layer[target].max(layer[node].saturating_add(1));
            indegree[target] = indegree[target].saturating_sub(1);
            if indegree[target] == 0 && !done[target] {
                queue.push(Reverse(target));
            }
        }
    }
    let remaining: Vec<usize> = (0..count).filter(|node| !done[*node]).collect();
    if remaining.is_empty() {
        return layer;
    }
    let remaining_set: BTreeSet<usize> = remaining.iter().copied().collect();
    let base = layer
        .iter()
        .enumerate()
        .filter(|(node, _)| !remaining_set.contains(node))
        .map(|(_, value)| value.saturating_add(1))
        .max()
        .unwrap_or(0);
    for node in &remaining {
        let predecessor = incoming[*node]
            .iter()
            .filter(|source| !remaining_set.contains(source))
            .map(|source| layer[*source].saturating_add(1))
            .max()
            .unwrap_or(base);
        layer[*node] = predecessor.max(base);
    }
    for node in &remaining {
        for target in &outgoing[*node] {
            if remaining_set.contains(target) {
                layer[*target] = layer[*target].max(layer[*node].saturating_add(1));
            }
        }
    }
    layer
}

fn node_width(node: &ResourceNode) -> i32 {
    let label = format!("{} {}", short_category(node.category), node.name);
    let width = label.width() as i32 + 4;
    width.clamp(MIN_NODE_WIDTH, MAX_NODE_WIDTH)
}

fn badges_for(node: &ResourceNode) -> Vec<String> {
    if let Some(scale) = tree::scale_badge(node) {
        return vec![scale];
    }
    node.badges
        .iter()
        .map(|badge| badge.label.clone())
        .take(1)
        .collect()
}

fn route(from: &LayoutNode, to: &LayoutNode) -> Vec<(i32, i32)> {
    let start = (from.x + from.width, from.y + 1);
    let end = (to.x - 1, to.y + 1);
    let middle = if end.0 > start.0 {
        (start.0 + end.0) / 2
    } else {
        start.0.saturating_add(2)
    };
    let mut points = vec![start, (middle, start.1), (middle, end.1), end];
    points.dedup();
    points
}

/// Renders the graph pane.
pub fn render(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    let theme = theme::knect_dark();
    let focused = state.active_pane == crate::app::state::Pane::Graph;
    let node_count = state
        .topology
        .as_ref()
        .map_or(0, |topology| topology.nodes.len());
    let edge_count = state
        .topology
        .as_ref()
        .map_or(0, |topology| topology.edges.len());
    let border = Block::bordered().border_style(theme.pane_border(focused));
    let inner = border.inner(area);
    let Some(layout) = state.graph_layout.as_deref() else {
        let title = Line::from(Span::styled(
            "Topology",
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        ));
        frame.render_widget(border.title(title), area);
        frame.render_widget(
            Paragraph::new("no topology yet — waiting for daemon events")
                .style(Style::default().fg(theme.muted)),
            inner,
        );
        return;
    };
    let compact = layout.width > i32::from(inner.width);
    let title = Line::from(vec![
        Span::styled(
            if compact {
                "Topology (compact) "
            } else {
                "Topology "
            },
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("{node_count} nodes · {edge_count} edges"),
            Style::default().fg(theme.muted),
        ),
    ]);
    frame.render_widget(border.title(title), area);
    if layout.nodes.is_empty() {
        frame.render_widget(
            Paragraph::new("topology is empty — no Knative resources discovered")
                .style(Style::default().fg(theme.muted)),
            inner,
        );
        return;
    }
    if compact {
        render_compact(frame, inner, state, layout);
        return;
    }
    let offset = clamp_offset(layout, inner, state.graph_offset);
    frame.render_widget(
        DagWidget {
            layout,
            offset,
            selected: state.selected.as_ref(),
            ascii: state.ascii,
            focused,
            theme,
        },
        inner,
    );
}

fn render_compact(frame: &mut Frame<'_>, area: Rect, state: &AppState, layout: &GraphLayout) {
    let theme = theme::knect_dark();
    let height = usize::from(area.height);
    let offset = (state.graph_offset.1.max(0) as usize).min(layout.nodes.len().saturating_sub(1));
    let mut lines = Vec::new();
    for (index, node) in layout.nodes.iter().enumerate().skip(offset).take(height) {
        let indent = "  ".repeat(node.layer.min(8));
        let selected = state.selected.as_ref() == Some(&node.id);
        let style = if selected {
            theme.selection()
        } else {
            Style::default().fg(theme.text)
        };
        let mut label = format!("{indent}▶ {} {}", node.label, health_tag(node.health));
        let health = health_tag(node.health);
        let badges: Vec<&String> = node
            .badges
            .iter()
            .filter(|badge| !badge.eq_ignore_ascii_case(health))
            .collect();
        if !badges.is_empty() {
            let joined = badges
                .iter()
                .map(|badge| badge.as_str())
                .collect::<Vec<_>>()
                .join(",");
            label.push_str(&format!(" [{joined}]"));
        }
        lines.push(Line::from(vec![
            Span::styled(
                format!("#{:02} ", index + 1),
                Style::default().fg(theme.muted),
            ),
            Span::styled(label, style),
        ]));
    }
    frame.render_widget(Paragraph::new(Text::from(lines)), area);
}

fn health_tag(health: Health) -> &'static str {
    match health {
        Health::Healthy => "healthy",
        Health::Degraded => "degraded",
        Health::Progressing => "progressing",
        Health::Blocked => "blocked",
        Health::Unknown => "unknown",
    }
}

fn clamp_offset(layout: &GraphLayout, area: Rect, requested: (i32, i32)) -> (i32, i32) {
    let max_x = layout.width.saturating_sub(i32::from(area.width)).max(0);
    let max_y = layout.height.saturating_sub(i32::from(area.height)).max(0);
    (requested.0.clamp(0, max_x), requested.1.clamp(0, max_y))
}

struct DagWidget<'a> {
    layout: &'a GraphLayout,
    offset: (i32, i32),
    selected: Option<&'a ResourceId>,
    ascii: bool,
    focused: bool,
    theme: theme::Theme,
}

impl Widget for DagWidget<'_> {
    fn render(self, area: Rect, buffer: &mut Buffer) {
        for edge in &self.layout.edges {
            draw_edge(buffer, area, edge, self.offset, self.ascii, &self.theme);
        }
        for node in &self.layout.nodes {
            let selected = self.selected == Some(&node.id);
            draw_node(
                buffer,
                area,
                node,
                self.offset,
                NodePaint {
                    selected,
                    focused: self.focused,
                    ascii: self.ascii,
                    theme: &self.theme,
                },
            );
        }
    }
}

fn draw_edge(
    buffer: &mut Buffer,
    area: Rect,
    edge: &LayoutEdge,
    offset: (i32, i32),
    ascii: bool,
    theme: &theme::Theme,
) {
    let glyphs = Glyphs::new(ascii);
    let color = theme.health(edge.health);
    let style = Style::default().fg(color);
    let points: Vec<(i32, i32)> = edge
        .points
        .iter()
        .map(|(x, y)| (x - offset.0, y - offset.1))
        .collect();
    for pair in points.windows(2) {
        draw_segment(buffer, area, pair[0], pair[1], glyphs, style);
    }
    for index in 1..points.len().saturating_sub(1) {
        let previous = direction(points[index - 1], points[index]);
        let next = direction(points[index], points[index + 1]);
        let glyph = corner(previous, next, ascii);
        draw_cell(buffer, area, points[index], glyph, style);
    }
    if let Some(&end) = points.last() {
        draw_cell(
            buffer,
            area,
            end,
            glyphs.arrow,
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        );
    }
}

fn direction(from: (i32, i32), to: (i32, i32)) -> (i32, i32) {
    ((to.0 - from.0).signum(), (to.1 - from.1).signum())
}

fn corner(previous: (i32, i32), next: (i32, i32), ascii: bool) -> &'static str {
    if ascii {
        return "+";
    }
    match (previous, next) {
        ((1, 0), (0, 1)) | ((0, -1), (-1, 0)) => "┐",
        ((1, 0), (0, -1)) | ((0, 1), (-1, 0)) => "┘",
        ((-1, 0), (0, 1)) | ((0, -1), (1, 0)) => "┌",
        ((-1, 0), (0, -1)) | ((0, 1), (1, 0)) => "└",
        _ => "┼",
    }
}

fn draw_segment(
    buffer: &mut Buffer,
    area: Rect,
    from: (i32, i32),
    to: (i32, i32),
    glyphs: Glyphs,
    style: Style,
) {
    let dx = (to.0 - from.0).signum();
    let dy = (to.1 - from.1).signum();
    if dx == 0 && dy == 0 {
        return;
    }
    let glyph = if dy == 0 {
        glyphs.horizontal
    } else {
        glyphs.vertical
    };
    let mut x = from.0;
    let mut y = from.1;
    loop {
        draw_cell(buffer, area, (x, y), glyph, style);
        if x == to.0 && y == to.1 {
            break;
        }
        x += dx;
        y += dy;
    }
}

fn draw_cell(buffer: &mut Buffer, area: Rect, point: (i32, i32), glyph: &str, style: Style) {
    let Ok(x) = u16::try_from(point.0) else {
        return;
    };
    let Ok(y) = u16::try_from(point.1) else {
        return;
    };
    if x >= area.width || y >= area.height {
        return;
    }
    let cell = &mut buffer[(area.x + x, area.y + y)];
    cell.set_symbol(glyph);
    cell.set_style(style);
}

#[derive(Debug, Clone, Copy)]
struct Glyphs {
    horizontal: &'static str,
    vertical: &'static str,
    arrow: &'static str,
    top_left: &'static str,
    top_right: &'static str,
    bottom_left: &'static str,
    bottom_right: &'static str,
}

impl Glyphs {
    const fn new(ascii: bool) -> Self {
        if ascii {
            Self {
                horizontal: "-",
                vertical: "|",
                arrow: ">",
                top_left: "+",
                top_right: "+",
                bottom_left: "+",
                bottom_right: "+",
            }
        } else {
            Self {
                horizontal: "─",
                vertical: "│",
                arrow: "▶",
                top_left: "┌",
                top_right: "┐",
                bottom_left: "└",
                bottom_right: "┘",
            }
        }
    }
}

struct NodePaint<'a> {
    selected: bool,
    focused: bool,
    ascii: bool,
    theme: &'a theme::Theme,
}

fn draw_node(
    buffer: &mut Buffer,
    area: Rect,
    node: &LayoutNode,
    offset: (i32, i32),
    paint: NodePaint<'_>,
) {
    let glyphs = Glyphs::new(paint.ascii);
    let theme = paint.theme;
    let selected = paint.selected;
    let focused = paint.focused;
    let x = node.x - offset.0;
    let y = node.y - offset.1;
    if x >= i32::from(area.width) || y >= i32::from(area.height) {
        return;
    }
    if x + node.width <= 0 || y + node.height <= 0 {
        return;
    }
    let inner_width = (node.width - 2).max(1) as usize;
    let border_style = if selected && focused {
        Style::default()
            .fg(theme.focus)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(theme.health(node.health))
    };
    let label_style = if selected {
        theme.selection()
    } else {
        Style::default().fg(theme.text)
    };
    let mut label = node.label.clone();
    if !node.badges.is_empty() {
        label.push_str(" [");
        label.push_str(&node.badges.join(","));
        label.push(']');
    }
    let label = fit(&label, inner_width);
    let top = format!(
        "{}{}{}",
        glyphs.top_left,
        glyphs.horizontal.repeat(inner_width),
        glyphs.top_right
    );
    let middle = format!(
        "{}{}{}",
        glyphs.vertical,
        pad(&label, inner_width),
        glyphs.vertical
    );
    let bottom = format!(
        "{}{}{}",
        glyphs.bottom_left,
        glyphs.horizontal.repeat(inner_width),
        glyphs.bottom_right
    );
    draw_line(buffer, area, x, y, &top, border_style);
    draw_line(buffer, area, x, y + 1, &middle, label_style);
    draw_line(buffer, area, x, y + 2, &bottom, border_style);
}

fn draw_line(buffer: &mut Buffer, area: Rect, x: i32, y: i32, text: &str, style: Style) {
    let Ok(screen_y) = u16::try_from(y) else {
        return;
    };
    if screen_y >= area.height {
        return;
    }
    let clip_left = x.min(0).unsigned_abs() as usize;
    let screen_x = x.max(0) as u16;
    if screen_x >= area.width {
        return;
    }
    let available = usize::from(area.width - screen_x);
    let clipped = clip_columns(text, clip_left, available);
    if clipped.is_empty() {
        return;
    }
    buffer.set_stringn(
        area.x + screen_x,
        area.y + screen_y,
        clipped,
        available,
        style,
    );
}

fn clip_columns(text: &str, skip: usize, max: usize) -> String {
    let mut output = String::new();
    let mut column = 0_usize;
    let mut width = 0_usize;
    for character in text.chars() {
        let character_width = character.width().unwrap_or(0);
        let next = column + character_width;
        if next <= skip {
            column = next;
            continue;
        }
        if width + character_width > max {
            break;
        }
        width += character_width;
        column = next;
        output.push(character);
    }
    output
}

fn fit(text: &str, width: usize) -> String {
    if text.width() <= width {
        return text.to_owned();
    }
    let mut output = String::new();
    let mut used = 0_usize;
    for character in text.chars() {
        let character_width = character.width().unwrap_or(0);
        if used + character_width > width.saturating_sub(1) {
            break;
        }
        used += character_width;
        output.push(character);
    }
    output.push('…');
    output
}

fn pad(text: &str, width: usize) -> String {
    let mut output = text.to_owned();
    let current = text.width();
    if current < width {
        output.push_str(&" ".repeat(width - current));
    }
    output
}

fn identity_key(id: &ResourceId) -> String {
    format!(
        "{}|{}|{}|{}",
        id.api_version,
        id.kind,
        id.namespace.as_deref().unwrap_or("-"),
        id.name
    )
}

/// Short display label for a node category.
#[must_use]
pub fn short_category(category: NodeCategory) -> &'static str {
    match category {
        NodeCategory::KService => "KService",
        NodeCategory::Route => "Route",
        NodeCategory::Configuration => "Config",
        NodeCategory::Revision => "Revision",
        NodeCategory::PodAutoscaler => "PA",
        NodeCategory::Broker => "Broker",
        NodeCategory::Trigger => "Trigger",
        NodeCategory::Source => "Source",
        NodeCategory::SinkBinding => "SinkBinding",
        NodeCategory::Subscription => "Subscription",
        NodeCategory::Channel => "Channel",
        NodeCategory::Deployment => "Deployment",
        NodeCategory::ReplicaSet => "ReplicaSet",
        NodeCategory::Pod => "Pod",
        NodeCategory::Service => "Service",
        NodeCategory::Endpoints => "Endpoints",
        NodeCategory::BackingService => "Backing",
        NodeCategory::Topic => "Topic",
        NodeCategory::StorageClass => "StorageClass",
        NodeCategory::PersistentVolume => "PV",
        NodeCategory::PersistentVolumeClaim => "PVC",
        NodeCategory::CephPool => "CephPool",
        NodeCategory::Policy => "Policy",
        NodeCategory::Canary => "Canary",
        NodeCategory::VirtualService => "VirtualService",
        NodeCategory::DestinationRule => "DestinationRule",
        NodeCategory::PeerAuthentication => "PeerAuth",
        NodeCategory::AuthorizationPolicy => "AuthzPolicy",
        NodeCategory::ConfigMap => "ConfigMap",
        NodeCategory::Secret => "Secret",
        NodeCategory::Namespace => "Namespace",
        NodeCategory::Unknown => "Resource",
    }
}

/// Exposed for tests and callers that want to render an edge label.
#[must_use]
pub fn edge_label(edge: &Edge) -> String {
    edge.label
        .clone()
        .unwrap_or_else(|| format!("{:?}", edge.kind).to_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use knect_proto::graph::{TopologyStats, Warning};

    fn node(kind: &str, name: &str) -> ResourceNode {
        ResourceNode {
            id: ResourceId::namespaced("example.test/v1", kind, "default", name),
            category: match kind {
                "Service" => NodeCategory::KService,
                "Revision" => NodeCategory::Revision,
                "Broker" => NodeCategory::Broker,
                "Trigger" => NodeCategory::Trigger,
                _ => NodeCategory::Unknown,
            },
            name: name.to_owned(),
            namespace: Some("default".to_owned()),
            health: Health::Healthy,
            summary: String::new(),
            labels: BTreeMap::new(),
            annotations: BTreeMap::new(),
            badges: Vec::new(),
            uid: None,
            first_seen: Utc::now(),
            last_seen: Utc::now(),
        }
    }

    fn edge(id: &str, from: &ResourceNode, to: &ResourceNode) -> Edge {
        Edge {
            id: id.to_owned(),
            from: from.id.clone(),
            to: to.id.clone(),
            kind: EdgeKind::Owns,
            label: None,
            health: Health::Healthy,
            warnings: Vec::new(),
            metadata: BTreeMap::new(),
        }
    }

    fn topology(nodes: Vec<ResourceNode>, edges: Vec<Edge>) -> Topology {
        Topology {
            schema_version: 1,
            context: "test".to_owned(),
            generation: 1,
            stats: TopologyStats {
                node_count: nodes.len() as u64,
                edge_count: edges.len() as u64,
                ..TopologyStats::default()
            },
            nodes,
            edges,
            warnings: Vec::<Warning>::new(),
        }
    }

    fn sample() -> Topology {
        let service = node("Service", "hello");
        let revision = node("Revision", "hello-00001");
        let broker = node("Broker", "orders");
        let trigger = node("Trigger", "orders-to-hello");
        topology(
            vec![
                service.clone(),
                revision.clone(),
                broker.clone(),
                trigger.clone(),
            ],
            vec![
                edge("e1", &service, &revision),
                edge("e2", &trigger, &broker),
                edge("e3", &trigger, &service),
            ],
        )
    }

    #[test]
    fn layout_is_deterministic() {
        let first = layout(&sample());
        let second = layout(&sample());
        assert_eq!(first, second);
    }

    #[test]
    fn every_edge_runs_left_to_right_in_the_sample() {
        let layout = layout(&sample());
        let by_id: BTreeMap<&ResourceId, &LayoutNode> =
            layout.nodes.iter().map(|node| (&node.id, node)).collect();
        for edge in &layout.edges {
            let from = by_id.get(&edge.from).expect("from");
            let to = by_id.get(&edge.to).expect("to");
            assert!(from.layer < to.layer, "edge runs backwards");
        }
    }

    #[test]
    fn boxes_do_not_overlap() {
        let layout = layout(&sample());
        for (index, left) in layout.nodes.iter().enumerate() {
            for right in layout.nodes.iter().skip(index + 1) {
                let overlap_x = left.x < right.x + right.width && right.x < left.x + left.width;
                let overlap_y = left.y < right.y + right.height && right.y < left.y + left.height;
                assert!(
                    !(overlap_x && overlap_y),
                    "boxes overlap: {left:?} {right:?}"
                );
            }
        }
    }

    #[test]
    fn edges_start_and_end_at_node_ports() {
        let layout = layout(&sample());
        let by_id: BTreeMap<&ResourceId, &LayoutNode> =
            layout.nodes.iter().map(|node| (&node.id, node)).collect();
        for edge in &layout.edges {
            let from = by_id.get(&edge.from).expect("from");
            let to = by_id.get(&edge.to).expect("to");
            assert_eq!(
                edge.points.first(),
                Some(&(from.x + from.width, from.y + 1))
            );
            assert_eq!(edge.points.last(), Some(&(to.x - 1, to.y + 1)));
        }
    }

    #[test]
    fn cycles_still_produce_a_finite_layout() {
        let a = node("Service", "a");
        let b = node("Revision", "b");
        let topology = topology(
            vec![a.clone(), b.clone()],
            vec![edge("e1", &a, &b), edge("e2", &b, &a)],
        );
        let layout = layout(&topology);
        assert_eq!(layout.nodes.len(), 2);
        assert!(layout.width > 0);
        assert!(layout.height > 0);
        for node in &layout.nodes {
            assert!(node.layer < 4);
        }
    }

    #[test]
    fn empty_topology_has_no_canvas() {
        let layout = layout(&topology(Vec::new(), Vec::new()));
        assert!(layout.nodes.is_empty());
        assert!(layout.edges.is_empty());
        assert_eq!(layout.width, 0);
    }

    #[test]
    fn clipping_helpers_respect_wide_characters() {
        assert_eq!(clip_columns("abcdef", 2, 3), "cde");
        assert_eq!(clip_columns("a한b", 1, 2), "한");
        assert_eq!(fit("abcdef", 4), "abc…");
        assert_eq!(pad("한", 3), "한 ");
    }
}
