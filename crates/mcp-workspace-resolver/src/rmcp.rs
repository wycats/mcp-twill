//! Conversions from `rmcp` model types into resolver observations.

use crate::observation::{McpRoot, McpRootsObservation};

impl From<rmcp::model::Root> for McpRoot {
    fn from(root: rmcp::model::Root) -> Self {
        Self {
            uri: root.uri,
            name: root.name,
        }
    }
}

impl From<&rmcp::model::Root> for McpRoot {
    fn from(root: &rmcp::model::Root) -> Self {
        Self {
            uri: root.uri.clone(),
            name: root.name.clone(),
        }
    }
}

impl From<rmcp::model::ListRootsResult> for McpRootsObservation {
    fn from(result: rmcp::model::ListRootsResult) -> Self {
        Self {
            roots: result.roots.into_iter().map(McpRoot::from).collect(),
        }
    }
}

impl From<&rmcp::model::ListRootsResult> for McpRootsObservation {
    fn from(result: &rmcp::model::ListRootsResult) -> Self {
        Self {
            roots: result.roots.iter().map(McpRoot::from).collect(),
        }
    }
}
