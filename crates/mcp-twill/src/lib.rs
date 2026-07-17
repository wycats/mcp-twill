pub mod argument_schemas;
pub mod builder;
pub mod catalog;
pub mod contract;
pub mod conversation_identity;
pub mod error;
pub mod event;
pub mod help;
pub mod model;
pub mod native_surfaces;
pub mod presentation;
pub mod registry;
pub mod resource;
pub mod response;
pub mod results;
pub mod rmcp_adapter;
pub mod runtime;
pub mod template;
pub mod types;

pub use argument_schemas::*;
pub use builder::*;
pub use catalog::*;
pub use contract::{
    ContractViolation, check_confirmation_projection, check_native_surface_projection,
    verify_catalog_coverage,
};
pub use conversation_identity::{
    CONVERSATION_IDENTITY_META_KEY, ConversationIdentity, InvocationContext,
};
pub use error::{FrameworkError, PreResolvedWorkspaceProblem, Result, WorkspaceMetadataProblem};
pub use event::{EventSink, FrameworkEvent, InMemoryEventSink, NoopEventSink, PlanFacts};
pub use help::{HelpDetail, HelpRequest, HelpResult, HelpTopic};
pub use mcp_workspace_resolver::{
    HostWorkspaceRoot, HostWorkspaceRootError, HostWorkspaceRootsObservation,
};
pub use model::*;
pub use native_surfaces::*;
pub use presentation::*;
pub use registry::{CommandHandler, CommandRegistry, HandlerFuture};
pub use resource::{
    Grant, Granted, Listed, Listing, ReadResource, Release, Res, ResolveResource,
    ResolvedResources, Resource, ResourceOutput, ResourceRefusal,
};
pub use response::*;
pub use results::*;
pub use rmcp_adapter::{
    CliMcpServer, CliMcpServerConfig, ConversationIdentityCompatibility,
    WorkspaceMetadataCompatibility,
};
pub use runtime::RuntimeIdentity;
pub use template::{CommandTemplate, TemplateToken};
pub use types::{Field, FieldShape, TypeDecl, Variant};
