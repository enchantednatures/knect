//! Modal state for mutation diffs, traffic splits, and CloudEvent taps.

mod diff;
mod seed;
mod tap;
mod traffic;

pub use diff::{DiffModal, SOURCES, pane_document, pane_lines};
pub use seed::{TrafficSeed, seed_traffic};
pub use tap::{TAP_EVENT_LIMIT, TapModal};
pub use traffic::{TrafficEdit, TrafficModal, TrafficRow};

/// Which modal is open.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ModalKind {
    /// No modal.
    #[default]
    None,
    /// Three-way mutation diff.
    Diff,
    /// Traffic-split editor.
    Traffic,
    /// CloudEvent tap.
    Tap,
}

/// Modal state.
#[derive(Debug, Clone, Default, PartialEq)]
pub enum Modal {
    /// No modal.
    #[default]
    None,
    /// Mutation diff.
    Diff(DiffModal),
    /// Traffic split editor.
    Traffic(TrafficModal),
    /// CloudEvent tap.
    Tap(TapModal),
}

impl Modal {
    /// Modal kind.
    #[must_use]
    pub const fn kind(&self) -> ModalKind {
        match self {
            Self::None => ModalKind::None,
            Self::Diff(_) => ModalKind::Diff,
            Self::Traffic(_) => ModalKind::Traffic,
            Self::Tap(_) => ModalKind::Tap,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use knect_proto::graph::{Edge, EdgeKind, Health, ResourceNode, Topology, TopologyStats};
    use knect_proto::tap::{TapEvent, TapTarget, TapTargetKind};
    use std::collections::BTreeMap;

    fn node(category: knect_proto::NodeCategory, kind: &str, name: &str) -> ResourceNode {
        ResourceNode {
            id: knect_proto::ResourceId::namespaced(
                "serving.knative.dev/v1",
                kind,
                "default",
                name,
            ),
            category,
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

    fn edge(id: &str, from: &ResourceNode, to: &ResourceNode, kind: EdgeKind) -> Edge {
        Edge {
            id: id.to_owned(),
            from: from.id.clone(),
            to: to.id.clone(),
            kind,
            label: None,
            health: Health::Healthy,
            warnings: Vec::new(),
            metadata: BTreeMap::new(),
        }
    }

    fn topology() -> (Topology, knect_proto::ResourceId) {
        let service = node(knect_proto::NodeCategory::KService, "Service", "hello");
        let route = node(knect_proto::NodeCategory::Route, "Route", "hello");
        let revision = node(
            knect_proto::NodeCategory::Revision,
            "Revision",
            "hello-00001",
        );
        let owns = edge("e1", &service, &route, EdgeKind::Owns);
        let mut traffic = edge("e2", &route, &revision, EdgeKind::RoutesTo);
        traffic
            .metadata
            .insert("percent".to_owned(), serde_json::json!(100));
        traffic
            .metadata
            .insert("tag".to_owned(), serde_json::json!("current"));
        (
            Topology {
                schema_version: 1,
                context: "test".to_owned(),
                generation: 1,
                nodes: vec![service.clone(), route, revision],
                edges: vec![owns, traffic],
                warnings: Vec::new(),
                stats: TopologyStats::default(),
            },
            service.id,
        )
    }

    #[test]
    fn traffic_seed_resolves_route_edges() {
        let (topology, service) = topology();
        let seed = seed_traffic(&topology, &service).expect("seed");
        assert_eq!(seed.rows.len(), 1);
        assert_eq!(seed.rows[0].revision, "hello-00001");
        assert_eq!(seed.rows[0].percent, 100);
        assert_eq!(seed.rows[0].tag, "current");
        assert!(seed.route.is_some());
    }

    #[test]
    fn traffic_validation_requires_a_legal_split() {
        let (topology, service) = topology();
        let mut modal = TrafficModal::from_topology(&topology, &service);
        assert!(modal.validate().is_ok());
        modal.rows[0].percent = 90;
        assert!(modal.validate().expect_err("sum").contains("sum to 100"));
        modal.rows[0].percent = 100;
        modal.rows[0].tag = "Bad Tag".to_owned();
        assert!(modal.validate().expect_err("tag").contains("lowercase"));
        modal.rows[0].tag = String::new();
        modal.rows.push(TrafficRow {
            revision: "hello-00001".to_owned(),
            percent: 0,
            tag: String::new(),
        });
        assert!(
            modal
                .validate()
                .expect_err("duplicate")
                .contains("more than once")
        );
    }

    #[test]
    fn editing_percentages_adjusts_and_commits() {
        let (topology, service) = topology();
        let mut modal = TrafficModal::from_topology(&topology, &service);
        modal.adjust(-40);
        assert_eq!(modal.rows[0].percent, 60);
        modal.adjust(500);
        assert_eq!(modal.rows[0].percent, 100);
        modal.edit_percent('4');
        modal.push_char('2');
        assert_eq!(modal.sum(), 42);
        modal.commit_edit().expect("commit");
        assert_eq!(modal.rows[0].percent, 42);
        modal.edit_tag();
        modal.push_char('n');
        modal.push_char('x');
        modal.commit_edit().expect("tag");
        assert_eq!(modal.rows[0].tag, "currentnx");
    }

    #[test]
    fn tap_buffer_is_bounded() {
        let mut modal = TapModal::starting(TapTarget {
            kind: TapTargetKind::Broker,
            namespace: "default".to_owned(),
            name: "orders".to_owned(),
        });
        for index in 0..(TAP_EVENT_LIMIT + 25) {
            modal.push_event(TapEvent {
                id: index.to_string(),
                ce_id: format!("ce-{index}"),
                ce_source: "test".to_owned(),
                ce_type: "test.event".to_owned(),
                ce_subject: None,
                ce_time: None,
                headers: BTreeMap::new(),
                payload: serde_json::json!({"index": index}),
                received_at: Utc::now(),
            });
        }
        assert_eq!(modal.events.len(), TAP_EVENT_LIMIT);
        assert_eq!(modal.selected, TAP_EVENT_LIMIT - 1);
        assert!(modal.selected_event().is_some());
    }
}
