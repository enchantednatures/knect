//! Stable identifiers for Kubernetes resources and field paths.

use std::fmt;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// A Kubernetes group, version, and kind.
#[derive(
    Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
pub struct Gvk {
    /// API group. Empty for the core (`v1`) group.
    pub group: String,
    /// API version, for example `v1` or `v1beta1`.
    pub version: String,
    /// Resource kind, for example `Service` or `Revision`.
    pub kind: String,
}

impl Gvk {
    /// Creates a GVK from its parts.
    #[must_use]
    pub fn new(
        group: impl Into<String>,
        version: impl Into<String>,
        kind: impl Into<String>,
    ) -> Self {
        Self {
            group: group.into(),
            version: version.into(),
            kind: kind.into(),
        }
    }

    /// Parses a Kubernetes `apiVersion` value and a kind.
    ///
    /// `v1` becomes group `""`/version `v1`; `serving.knative.dev/v1` becomes
    /// group `serving.knative.dev`/version `v1`.
    #[must_use]
    pub fn from_api_version(api_version: &str, kind: impl Into<String>) -> Self {
        match api_version.split_once('/') {
            Some((group, version)) => Self::new(group, version, kind),
            None => Self::new("", api_version, kind),
        }
    }

    /// Returns the canonical `apiVersion` string (`group/version` or `version`).
    #[must_use]
    pub fn api_version(&self) -> String {
        if self.group.is_empty() {
            self.version.clone()
        } else {
            format!("{}/{}", self.group, self.version)
        }
    }

    /// Returns true for resources in the core API group.
    #[must_use]
    pub fn is_core(&self) -> bool {
        self.group.is_empty()
    }
}

impl fmt::Display for Gvk {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.api_version(), self.kind)
    }
}

/// The canonical identity of a Kubernetes object.
///
/// Node IDs in the topology are `ResourceId` values. The Kubernetes UID is
/// deliberately not part of the identity so a recreated object keeps its graph
/// position; the UID is carried separately in [`crate::graph::ResourceNode`].
#[derive(
    Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
pub struct ResourceId {
    /// Kubernetes `apiVersion`, for example `serving.knative.dev/v1`.
    pub api_version: String,
    /// Resource kind, for example `Service`.
    pub kind: String,
    /// Namespace, or `None` for cluster-scoped resources.
    pub namespace: Option<String>,
    /// Object name.
    pub name: String,
}

impl ResourceId {
    /// Creates an identifier, which may be namespace-scoped or cluster-scoped.
    #[must_use]
    pub fn new(
        api_version: impl Into<String>,
        kind: impl Into<String>,
        namespace: Option<String>,
        name: impl Into<String>,
    ) -> Self {
        Self {
            api_version: api_version.into(),
            kind: kind.into(),
            namespace,
            name: name.into(),
        }
    }

    /// Creates a namespaced identifier.
    #[must_use]
    pub fn namespaced(
        api_version: impl Into<String>,
        kind: impl Into<String>,
        namespace: impl Into<String>,
        name: impl Into<String>,
    ) -> Self {
        Self::new(api_version, kind, Some(namespace.into()), name)
    }

    /// Creates a cluster-scoped identifier.
    #[must_use]
    pub fn cluster_scoped(
        api_version: impl Into<String>,
        kind: impl Into<String>,
        name: impl Into<String>,
    ) -> Self {
        Self::new(api_version, kind, None, name)
    }

    /// Returns the GVK of this resource.
    #[must_use]
    pub fn gvk(&self) -> Gvk {
        Gvk::from_api_version(&self.api_version, self.kind.clone())
    }

    /// Returns true when this identifier has no namespace.
    #[must_use]
    pub fn is_cluster_scoped(&self) -> bool {
        self.namespace.is_none()
    }
}

impl fmt::Display for ResourceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.namespace {
            Some(namespace) => write!(
                f,
                "{}/{}/{}/{}",
                self.api_version, self.kind, namespace, self.name
            ),
            None => write!(f, "{}/{}/{}", self.api_version, self.kind, self.name),
        }
    }
}

/// A JSON Pointer (RFC 6901) locating a field inside a resource document.
#[derive(
    Debug, Clone, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(transparent)]
pub struct FieldPath(String);

impl FieldPath {
    /// Creates a field path from a JSON Pointer string.
    #[must_use]
    pub fn new(path: impl Into<String>) -> Self {
        Self(path.into())
    }

    /// Returns the underlying JSON Pointer.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Consumes the path and returns the underlying string.
    #[must_use]
    pub fn into_string(self) -> String {
        self.0
    }

    /// Returns true for the document root (the empty pointer).
    #[must_use]
    pub fn is_root(&self) -> bool {
        self.0.is_empty()
    }

    /// Iterates over unescaped path segments, skipping the root marker.
    pub fn segments(&self) -> impl Iterator<Item = &str> {
        self.0.split('/').filter(|segment| !segment.is_empty())
    }
}

impl fmt::Display for FieldPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<&str> for FieldPath {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl From<String> for FieldPath {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl AsRef<str> for FieldPath {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}
