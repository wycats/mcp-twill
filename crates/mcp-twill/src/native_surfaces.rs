use std::{
    cmp::Ordering,
    collections::{BTreeMap, BTreeSet},
    error::Error,
    fmt,
    sync::Arc,
};

use async_trait::async_trait;
use rmcp::{
    handler::server::tool::schema_for_type,
    model::{JsonObject, TaskSupport, Tool, ToolAnnotations, ToolExecution},
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

use crate::{
    CommandRegistry, FrameworkError, HelpRequest, HelpResult, OperationSpec, PermissionPreview,
    PreparedConfirmation, Result, ServingSurfaceIdentity, SurfacePresentationDefaults,
};

const SNAPSHOT_VERSION: u32 = 1;
const NATIVE_HASH_DOMAIN: &str = "io.github.wycats.mcp-twill/native-tool-surface";
const EFFECT_LANE_HASH_DOMAIN: &str = "io.github.wycats.mcp-twill/effect-lane-surface";
type NativeRouteProjection = BTreeMap<String, (String, Option<(String, String)>)>;

fn build_error(message: impl Into<String>) -> FrameworkError {
    FrameworkError::Build(message.into())
}

#[derive(Debug, Clone)]
pub enum McpToolSurface {
    EffectLanes(EffectLaneSurface),
    Native(NativeToolSurface),
}

impl From<NativeToolSurface> for McpToolSurface {
    fn from(surface: NativeToolSurface) -> Self {
        Self::Native(surface)
    }
}

#[derive(Debug, Clone)]
pub struct EffectLaneSurface {
    pub(crate) snapshot: EffectLaneSurfaceSnapshot,
}

#[derive(Debug, Clone)]
pub(crate) struct EffectLaneSurfaceSnapshot {
    pub(crate) identity: ServingSurfaceIdentity,
    pub(crate) document: Value,
    pub(crate) tools: Vec<Tool>,
    pub(crate) instructions: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum McpProtocolTarget {
    V2025_11_25,
    V2026_06_30,
}

impl McpProtocolTarget {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::V2025_11_25 => "2025-11-25",
            Self::V2026_06_30 => "2026-06-30",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct NativeToolSurfaceDecl {
    pub name: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<NativeToolDecl>,
    #[serde(default, skip_serializing_if = "NativeExposurePolicy::is_complete")]
    pub exposure: NativeExposurePolicy,
    pub framework_help: FrameworkHelpProjection,
    #[serde(
        default,
        skip_serializing_if = "NativeApplicationErrorDialect::is_canonical"
    )]
    pub application_errors: NativeApplicationErrorDialect,
    pub confirmation: NativeConfirmationRoute,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub resource_bindings: Vec<crate::ResourceBindingDecl>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum NativeExposurePolicy {
    #[default]
    Complete,
    ExplicitSubset {
        omitted_operations: BTreeSet<String>,
    },
}

impl NativeExposurePolicy {
    pub fn explicit_subset(omitted_operations: impl IntoIterator<Item = impl AsRef<str>>) -> Self {
        let omitted_operations = omitted_operations
            .into_iter()
            .map(|operation| operation.as_ref().to_string())
            .collect::<BTreeSet<_>>();
        if omitted_operations.is_empty() {
            Self::Complete
        } else {
            Self::ExplicitSubset { omitted_operations }
        }
    }

    pub(crate) fn is_complete(&self) -> bool {
        matches!(self, Self::Complete)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum FrameworkHelpProjection {
    Omitted,
    Tool { name: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "camelCase")]
pub enum NativeApplicationErrorDialect {
    #[default]
    Canonical,
    FlatSingleRecovery,
}

impl NativeApplicationErrorDialect {
    pub(crate) fn is_canonical(&self) -> bool {
        matches!(self, Self::Canonical)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum NativeConfirmationRoute {
    Bridge,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum NativeToolDecl {
    Direct {
        name: String,
        operation_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        description: Option<String>,
    },
    Group {
        name: String,
        selector: String,
        members: Vec<NativeToolMember>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        description: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct NativeToolMember {
    pub selector_value: String,
    pub operation_id: String,
}

#[derive(Clone)]
pub struct NativeToolSurface {
    declaration: NativeToolSurfaceDecl,
    snapshot: Box<NativeToolSurfaceSnapshot>,
    routes: BTreeMap<String, CompiledNativeRoute>,
    resource_binders: BTreeMap<String, Arc<dyn crate::ambient_resources::ErasedAmbientBinder>>,
}

impl fmt::Debug for NativeToolSurface {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NativeToolSurface")
            .field("declaration", &self.declaration)
            .field("snapshot", &self.snapshot)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone)]
enum CompiledNativeRoute {
    Direct {
        operation_id: String,
    },
    Group {
        selector: String,
        members: BTreeMap<String, String>,
    },
}

impl NativeToolSurface {
    pub fn builder(name: impl Into<String>) -> NativeToolSurfaceBuilder {
        NativeToolSurfaceBuilder::new(name.into())
    }

    pub fn builder_from(declaration: NativeToolSurfaceDecl) -> NativeToolSurfaceBuilder {
        NativeToolSurfaceBuilder::from_declaration(declaration)
    }

    pub fn declaration(&self) -> &NativeToolSurfaceDecl {
        &self.declaration
    }

    pub fn snapshot(&self) -> &NativeToolSurfaceSnapshot {
        &self.snapshot
    }

    pub(crate) fn identity(&self) -> Result<ServingSurfaceIdentity> {
        ServingSurfaceIdentity::new(
            self.snapshot.name.clone(),
            self.snapshot.surface_hash.clone(),
        )
    }

    pub(crate) fn confirmation_route(&self) -> NativeConfirmationRoute {
        self.declaration.confirmation
    }

    pub(crate) fn presentation_defaults(
        &self,
        operation_id: &str,
    ) -> Option<&SurfacePresentationDefaults> {
        self.snapshot
            .operation(operation_id)
            .map(NativeSurfaceOperation::presentation_defaults)
    }

    pub(crate) fn resource_binding(&self, resource: &str) -> Option<&crate::ResourceBindingDecl> {
        self.declaration
            .resource_bindings
            .iter()
            .find(|binding| binding.resource == resource)
    }

    pub(crate) fn resource_binder(
        &self,
        resource: &str,
    ) -> Option<Arc<dyn crate::ambient_resources::ErasedAmbientBinder>> {
        self.resource_binders.get(resource).cloned()
    }

    pub(crate) fn help(&self, request: HelpRequest) -> HelpResult {
        if let Some(name) = request.command {
            let Some(tool) = self.snapshot.tools.iter().find(|tool| tool.name == name) else {
                return HelpResult {
                    title: "Unknown native tool".to_string(),
                    text: format!("Unknown native tool `{name}`."),
                    structured: json!({ "tool": name, "found": false }),
                };
            };
            let mut text = format!(
                "# `{}`\n\n{}",
                tool.name,
                tool.description.as_deref().unwrap_or("")
            );
            text.push_str(&format!(
                "\n\nInput schema:\n`{}`\n\nOutput schema:\n`{}`",
                Value::Object((*tool.input_schema).clone()),
                tool.output_schema
                    .as_ref()
                    .map(|schema| Value::Object((**schema).clone()))
                    .unwrap_or(Value::Null)
            ));
            return HelpResult {
                title: tool
                    .annotations
                    .as_ref()
                    .and_then(|annotations| annotations.title.clone())
                    .unwrap_or_else(|| tool.name.to_string()),
                text,
                structured: serde_json::to_value(tool).unwrap_or(Value::Null),
            };
        }
        let text = self
            .snapshot
            .tools
            .iter()
            .map(|tool| {
                format!(
                    "- `{}` — {}",
                    tool.name,
                    tool.description.as_deref().unwrap_or("")
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        HelpResult {
            title: self.snapshot.name.clone(),
            text: format!("# {}\n\n{}", self.snapshot.name, text),
            structured: json!({
                "surface": {
                    "name": self.snapshot.name,
                    "hash": self.snapshot.surface_hash,
                },
                "tools": self.snapshot.tools,
                "operations": self.snapshot.operations.iter().map(operation_document).collect::<Result<Vec<_>>>().unwrap_or_default(),
            }),
        }
    }

    pub(crate) fn resolve_call(
        &self,
        tool: &str,
        mut arguments: JsonObject,
    ) -> Result<(String, JsonObject)> {
        match self.routes.get(tool) {
            Some(CompiledNativeRoute::Direct { operation_id }) => {
                Ok((operation_id.clone(), arguments))
            }
            Some(CompiledNativeRoute::Group { selector, members }) => {
                let selected = arguments
                    .remove(selector)
                    .ok_or_else(|| FrameworkError::MissingArgument(selector.clone()))?;
                let selected = selected.as_str().map(ToOwned::to_owned).ok_or_else(|| {
                    FrameworkError::InvalidArgumentType(selector.clone(), "a string")
                })?;
                let operation_id = members.get(&selected).cloned().ok_or_else(|| {
                    FrameworkError::InvalidArgumentType(selector.clone(), "a declared group member")
                })?;
                Ok((operation_id, arguments))
            }
            None => Err(FrameworkError::UnknownCommand {
                command: tool.to_string(),
                nearest: Vec::new(),
            }),
        }
    }
}

pub struct NativeToolSurfaceBuilder {
    declaration: NativeToolSurfaceDecl,
    exposure_authored: bool,
    framework_help_authored: bool,
    application_errors_authored: bool,
    confirmation_authored: bool,
    seeded_resource_bindings: BTreeSet<String>,
    resource_binders: BTreeMap<String, Arc<dyn crate::ambient_resources::ErasedAmbientBinder>>,
    binder_error_footprints: BTreeMap<String, Vec<String>>,
    errors: Vec<FrameworkError>,
}

impl NativeToolSurfaceBuilder {
    fn new(name: String) -> Self {
        Self {
            declaration: NativeToolSurfaceDecl {
                name,
                tools: Vec::new(),
                exposure: NativeExposurePolicy::Complete,
                framework_help: FrameworkHelpProjection::Omitted,
                application_errors: NativeApplicationErrorDialect::Canonical,
                confirmation: NativeConfirmationRoute::Unavailable,
                resource_bindings: Vec::new(),
            },
            exposure_authored: false,
            framework_help_authored: false,
            application_errors_authored: false,
            confirmation_authored: false,
            seeded_resource_bindings: BTreeSet::new(),
            resource_binders: BTreeMap::new(),
            binder_error_footprints: BTreeMap::new(),
            errors: Vec::new(),
        }
    }

    fn from_declaration(mut declaration: NativeToolSurfaceDecl) -> Self {
        if matches!(
            &declaration.exposure,
            NativeExposurePolicy::ExplicitSubset { omitted_operations }
                if omitted_operations.is_empty()
        ) {
            declaration.exposure = NativeExposurePolicy::Complete;
        }
        let seeded_resource_bindings = declaration
            .resource_bindings
            .iter()
            .map(|binding| binding.resource.clone())
            .collect();
        Self {
            declaration,
            exposure_authored: true,
            framework_help_authored: true,
            application_errors_authored: true,
            confirmation_authored: true,
            seeded_resource_bindings,
            resource_binders: BTreeMap::new(),
            binder_error_footprints: BTreeMap::new(),
            errors: Vec::new(),
        }
    }

    pub fn exposure(mut self, policy: NativeExposurePolicy) -> Self {
        if self.exposure_authored {
            self.errors.push(build_error(
                "native surface assigns `exposure` more than once",
            ));
        } else {
            self.exposure_authored = true;
            self.declaration.exposure = match policy {
                NativeExposurePolicy::ExplicitSubset { omitted_operations }
                    if omitted_operations.is_empty() =>
                {
                    NativeExposurePolicy::Complete
                }
                policy => policy,
            };
        }
        self
    }

    pub fn framework_help(mut self, projection: FrameworkHelpProjection) -> Self {
        if self.framework_help_authored {
            self.errors.push(build_error(
                "native surface assigns `framework_help` more than once",
            ));
        } else {
            self.framework_help_authored = true;
            self.declaration.framework_help = projection;
        }
        self
    }

    pub fn application_errors(mut self, dialect: NativeApplicationErrorDialect) -> Self {
        if self.application_errors_authored {
            self.errors.push(build_error(
                "native surface assigns `application_errors` more than once",
            ));
        } else {
            self.application_errors_authored = true;
            self.declaration.application_errors = dialect;
        }
        self
    }

    pub fn confirmation_route(mut self, route: NativeConfirmationRoute) -> Self {
        if self.confirmation_authored {
            self.errors.push(build_error(
                "native surface assigns `confirmation_route` more than once",
            ));
        } else {
            self.confirmation_authored = true;
            self.declaration.confirmation = route;
        }
        self
    }

    pub fn tool(mut self, declaration: NativeToolDecl) -> Self {
        self.declaration.tools.push(declaration);
        self
    }

    pub fn bind_resource<T>(
        mut self,
        binding: crate::AmbientResourceBinding<impl crate::BindAmbientResource<T>>,
    ) -> Self
    where
        T: crate::Resource,
    {
        if self.seeded_resource_bindings.contains(T::NAME)
            || self
                .declaration
                .resource_bindings
                .iter()
                .any(|entry| entry.resource == T::NAME)
        {
            self.errors.push(build_error(format!(
                "native surface declares resource binding `{}` more than once",
                T::NAME
            )));
            return self;
        }
        match binding.into_runtime::<T>() {
            Ok((declaration, binder, codes)) => {
                self.declaration.resource_bindings.push(declaration);
                self.resource_binders.insert(T::NAME.to_string(), binder);
                self.binder_error_footprints
                    .insert(T::NAME.to_string(), codes);
            }
            Err(error) => self.errors.push(error),
        }
        self
    }

    pub fn attach_resource_binder<T>(mut self, binder: impl crate::BindAmbientResource<T>) -> Self
    where
        T: crate::Resource,
    {
        let declared = self
            .declaration
            .resource_bindings
            .iter()
            .find(|binding| binding.resource == T::NAME);
        if !matches!(
            declared.map(|binding| &binding.mode),
            Some(crate::ResourceBindingMode::Ambient { .. })
        ) {
            self.errors.push(build_error(format!(
                "native surface cannot attach a binder for undeclared ambient resource `{}`",
                T::NAME
            )));
            return self;
        }
        if self.resource_binders.contains_key(T::NAME) {
            self.errors.push(build_error(format!(
                "native surface attaches resource binder `{}` more than once",
                T::NAME
            )));
            return self;
        }
        let codes = ambient_binder_error_codes::<T, _>(&binder);
        self.resource_binders.insert(
            T::NAME.to_string(),
            Arc::new(crate::ambient_resources::AmbientBinderAdapter::<T, _>::new(
                binder,
            )),
        );
        self.binder_error_footprints.insert(
            T::NAME.to_string(),
            codes.into_iter().map(ToOwned::to_owned).collect(),
        );
        self
    }

    pub fn direct(mut self, name: impl Into<String>, operation_id: impl Into<String>) -> Self {
        self.declaration.tools.push(NativeToolDecl::Direct {
            name: name.into(),
            operation_id: operation_id.into(),
            title: None,
            description: None,
        });
        self
    }

    pub fn group(
        mut self,
        name: impl Into<String>,
        build: impl FnOnce(&mut NativeToolGroupBuilder),
    ) -> Self {
        let mut group = NativeToolGroupBuilder::new(name.into());
        build(&mut group);
        match group.finish() {
            Ok(declaration) => self.declaration.tools.push(declaration),
            Err(error) => self.errors.push(error),
        }
        self
    }

    pub fn build(
        mut self,
        registry: &CommandRegistry,
        target: McpProtocolTarget,
    ) -> Result<NativeToolSurface> {
        if let Some(error) = self.errors.into_iter().next() {
            return Err(error);
        }
        if !self.framework_help_authored {
            return Err(build_error(
                "native surface must explicitly select framework help projection",
            ));
        }
        if !self.confirmation_authored {
            return Err(build_error(
                "native surface must explicitly select a confirmation route",
            ));
        }
        if matches!(
            &self.declaration.exposure,
            NativeExposurePolicy::ExplicitSubset { omitted_operations }
                if omitted_operations.is_empty()
        ) {
            self.declaration.exposure = NativeExposurePolicy::Complete;
        }
        compile_native_surface(
            self.declaration,
            self.resource_binders,
            self.binder_error_footprints,
            registry,
            target,
        )
    }
}

fn ambient_binder_error_codes<T, B>(_binder: &B) -> Vec<&'static str>
where
    T: crate::Resource,
    B: crate::BindAmbientResource<T>,
{
    <B::ErrorFootprint as crate::ApplicationErrorFootprint<B::Error>>::codes()
}

pub struct NativeToolGroupBuilder {
    name: String,
    selector: Option<String>,
    members: Vec<NativeToolMember>,
    title: Option<String>,
    description: Option<String>,
    errors: Vec<FrameworkError>,
}

impl NativeToolGroupBuilder {
    fn new(name: String) -> Self {
        Self {
            name,
            selector: None,
            members: Vec::new(),
            title: None,
            description: None,
            errors: Vec::new(),
        }
    }

    pub fn selector(&mut self, argument: impl Into<String>) -> &mut Self {
        if self.selector.is_some() {
            self.errors.push(build_error(
                "native group assigns `selector` more than once",
            ));
        } else {
            self.selector = Some(argument.into());
        }
        self
    }

    pub fn member(
        &mut self,
        selector_value: impl Into<String>,
        operation_id: impl Into<String>,
    ) -> &mut Self {
        self.members.push(NativeToolMember {
            selector_value: selector_value.into(),
            operation_id: operation_id.into(),
        });
        self
    }

    pub fn title(&mut self, title: impl Into<String>) -> &mut Self {
        if self.title.is_some() {
            self.errors
                .push(build_error("native group assigns `title` more than once"));
        } else {
            self.title = Some(title.into());
        }
        self
    }

    pub fn description(&mut self, description: impl Into<String>) -> &mut Self {
        if self.description.is_some() {
            self.errors.push(build_error(
                "native group assigns `description` more than once",
            ));
        } else {
            self.description = Some(description.into());
        }
        self
    }

    fn finish(self) -> Result<NativeToolDecl> {
        if let Some(error) = self.errors.into_iter().next() {
            return Err(error);
        }
        Ok(NativeToolDecl::Group {
            name: self.name,
            selector: self
                .selector
                .ok_or_else(|| build_error("native group is missing its selector"))?,
            members: self.members,
            title: self.title,
            description: self.description,
        })
    }
}

#[derive(Debug, Clone)]
pub struct NativeToolSurfaceSnapshot {
    version: u32,
    protocol_version: String,
    name: String,
    catalog_hash: String,
    surface_hash: String,
    declaration: NativeToolSurfaceDecl,
    server_instructions: String,
    tools: Vec<Tool>,
    operations: Vec<NativeSurfaceOperation>,
    document: Value,
    canonical_json: Box<[u8]>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NativeSurfaceOperation {
    spec: OperationSpec,
    call: NativeSurfaceCall,
    presentation_defaults: SurfacePresentationDefaults,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NativeSurfaceCall {
    tool: String,
    arguments: Option<BTreeMap<String, Value>>,
}

impl NativeToolSurfaceSnapshot {
    pub fn version(&self) -> u32 {
        self.version
    }

    pub fn protocol_version(&self) -> &str {
        &self.protocol_version
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn catalog_hash(&self) -> &str {
        &self.catalog_hash
    }

    pub fn surface_hash(&self) -> &str {
        &self.surface_hash
    }

    pub fn document(&self) -> &Value {
        &self.document
    }

    pub fn canonical_json(&self) -> &[u8] {
        &self.canonical_json
    }

    pub fn declaration(&self) -> &NativeToolSurfaceDecl {
        &self.declaration
    }

    pub fn server_instructions(&self) -> &str {
        &self.server_instructions
    }

    pub fn tools(&self) -> &[Tool] {
        &self.tools
    }

    pub fn operations(&self) -> &[NativeSurfaceOperation] {
        &self.operations
    }

    pub fn operation(&self, operation_id: &str) -> Option<&NativeSurfaceOperation> {
        self.operations
            .iter()
            .find(|operation| operation.spec.id == operation_id)
    }
}

impl NativeSurfaceOperation {
    pub fn spec(&self) -> &OperationSpec {
        &self.spec
    }

    pub fn call(&self) -> &NativeSurfaceCall {
        &self.call
    }

    pub fn presentation_defaults(&self) -> &SurfacePresentationDefaults {
        &self.presentation_defaults
    }
}

impl NativeSurfaceCall {
    pub fn tool(&self) -> &str {
        &self.tool
    }

    pub fn arguments(&self) -> Option<&BTreeMap<String, Value>> {
        self.arguments.as_ref()
    }
}

#[async_trait]
pub trait NativeConfirmationBridge: Send + Sync + 'static {
    async fn confirm(
        &self,
        request: NativeConfirmationRequest,
    ) -> std::result::Result<NativeConfirmationDecision, NativeConfirmationBridgeError>;
}

pub struct NativeConfirmationRequest {
    preview: PermissionPreview,
    arguments: BTreeMap<String, Value>,
    invocation_fingerprint: String,
}

impl NativeConfirmationRequest {
    pub(crate) fn new(
        preview: PermissionPreview,
        arguments: BTreeMap<String, Value>,
        invocation_fingerprint: String,
    ) -> Self {
        Self {
            preview,
            arguments,
            invocation_fingerprint,
        }
    }

    pub fn preview(&self) -> &PermissionPreview {
        &self.preview
    }

    pub fn arguments(&self) -> &BTreeMap<String, Value> {
        &self.arguments
    }

    pub fn presentation(&self) -> &PreparedConfirmation {
        self.preview
            .confirmation
            .as_ref()
            .expect("native confirmation requests always contain presentation")
    }

    pub fn invocation_fingerprint(&self) -> &str {
        &self.invocation_fingerprint
    }
}

impl fmt::Debug for NativeConfirmationRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NativeConfirmationRequest")
            .field("preview", &self.preview)
            .field("arguments", &"<redacted>")
            .field("invocation_fingerprint", &self.invocation_fingerprint)
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeConfirmationDecision {
    Allow,
    Deny,
    Canceled,
}

pub struct NativeConfirmationBridgeError {
    _source: Box<dyn Error + Send + Sync + 'static>,
}

impl NativeConfirmationBridgeError {
    pub fn new(source: impl Error + Send + Sync + 'static) -> Self {
        Self {
            _source: Box::new(source),
        }
    }
}

impl fmt::Debug for NativeConfirmationBridgeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("NativeConfirmationBridgeError(<redacted>)")
    }
}

impl fmt::Display for NativeConfirmationBridgeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("native confirmation bridge failed")
    }
}

impl Error for NativeConfirmationBridgeError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        // The source is retained only for ownership and drop. Exposing it
        // would let host-private bridge diagnostics cross the framework
        // boundary through otherwise generic error reporting.
        None
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct NativeApplicationErrorBody {
    pub code: String,
    pub message: String,
    pub details: Value,
    pub recoveries: Vec<NativeApplicationRecovery>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum NativeApplicationRecovery {
    Tool {
        tool: String,
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        arguments: BTreeMap<String, Value>,
    },
    Action {
        code: String,
        summary: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct FlatNativeApplicationErrorBody {
    pub code: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recovery: Option<String>,
}

fn compile_native_surface(
    mut declaration: NativeToolSurfaceDecl,
    resource_binders: BTreeMap<String, Arc<dyn crate::ambient_resources::ErasedAmbientBinder>>,
    binder_error_footprints: BTreeMap<String, Vec<String>>,
    registry: &CommandRegistry,
    target: McpProtocolTarget,
) -> Result<NativeToolSurface> {
    validate_registry(registry)?;
    validate_surface_name(&declaration.name)?;

    let mut operation_specs = BTreeMap::new();
    for operation in registry.operation_specs() {
        if operation_specs
            .insert(operation.id.clone(), operation)
            .is_some()
        {
            return Err(build_error(
                "command paths produce a duplicate native operation id",
            ));
        }
    }
    let mut command_specs = BTreeMap::new();
    for command in registry.command_specs() {
        let operation_id = command.path.join(".");
        if command_specs.insert(operation_id, command).is_some() {
            return Err(build_error(
                "command paths produce a duplicate native operation id",
            ));
        }
    }
    let declared_routes = declaration_routes(&declaration);

    normalize_resource_bindings(
        &mut declaration,
        &command_specs,
        &resource_binders,
        &binder_error_footprints,
        registry,
    )?;
    let effective_bindings = declaration
        .resource_bindings
        .iter()
        .map(|binding| (binding.resource.clone(), binding.mode.clone()))
        .collect::<BTreeMap<_, _>>();

    let mut mapped_operations = BTreeSet::new();
    let mut tool_names = BTreeSet::new();
    let mut routes = BTreeMap::new();
    let mut tools = Vec::new();
    let mut operations = Vec::new();

    if let FrameworkHelpProjection::Tool { name } = &declaration.framework_help {
        validate_tool_name(name, "framework help tool")?;
        tool_names.insert(name.clone());
        tools.push(
            Tool::new(
                name.clone(),
                "Return catalog-derived help for the native serving surface.",
                schema_for_type::<HelpRequest>(),
            )
            .with_title("Help")
            .with_execution(ToolExecution::new().with_task_support(TaskSupport::Forbidden))
            .annotate(
                ToolAnnotations::new()
                    .read_only(true)
                    .destructive(false)
                    .idempotent(true)
                    .open_world(false),
            ),
        );
    }

    for tool_decl in &declaration.tools {
        match tool_decl {
            NativeToolDecl::Direct {
                name,
                operation_id,
                title,
                description,
            } => {
                validate_tool_name(name, "native tool")?;
                if !tool_names.insert(name.clone()) {
                    return Err(build_error(format!(
                        "native surface declares duplicate tool name `{name}`"
                    )));
                }
                let operation = operation_specs.get(operation_id).ok_or_else(|| {
                    build_error(format!(
                        "native tool `{name}` references unknown operation `{operation_id}`"
                    ))
                })?;
                let command = command_specs.get(operation_id).ok_or_else(|| {
                    build_error(format!(
                        "native tool `{name}` cannot resolve command `{operation_id}`"
                    ))
                })?;
                if !mapped_operations.insert(operation_id.clone()) {
                    return Err(build_error(format!(
                        "operation `{operation_id}` is mapped more than once"
                    )));
                }
                let output_schema = application_success_schema(operation)?;
                if !schema_accepts_objects_only(output_schema, output_schema)? {
                    return Err(build_error(format!(
                        "native direct tool `{name}` requires an object-only success schema"
                    )));
                }
                validate_application_error_dialect(
                    &declaration.application_errors,
                    operation,
                    &declaration,
                )?;
                let display_title = title.clone().unwrap_or_else(|| operation.summary.clone());
                let final_description = description
                    .clone()
                    .unwrap_or_else(|| operation.description.clone());
                let final_description = append_operation_guidance(
                    final_description,
                    operation,
                    &operation_specs,
                    &declared_routes,
                );
                let final_description = append_resource_binding_guidance(
                    final_description,
                    std::iter::once(*command),
                    registry,
                    &effective_bindings,
                );
                let defaults = presentation_defaults(&display_title)?;
                let mut input = schema_object(registry.arg_schema(command), "direct input schema")?;
                refine_input_schema_for_bindings(
                    &mut input,
                    command,
                    registry,
                    &effective_bindings,
                );
                let output = schema_object(output_schema.clone(), "direct output schema")?;
                let tool = Tool::new(name.clone(), final_description, input)
                    .with_title(display_title.clone())
                    .with_raw_output_schema(Arc::new(output))
                    .with_execution(ToolExecution::new().with_task_support(TaskSupport::Forbidden))
                    .annotate(annotations_for_operations(
                        std::slice::from_ref(operation),
                        &display_title,
                    ));
                tools.push(tool);
                operations.push(NativeSurfaceOperation {
                    spec: (*operation).clone(),
                    call: NativeSurfaceCall {
                        tool: name.clone(),
                        arguments: None,
                    },
                    presentation_defaults: defaults,
                });
                routes.insert(
                    name.clone(),
                    CompiledNativeRoute::Direct {
                        operation_id: operation_id.clone(),
                    },
                );
            }
            NativeToolDecl::Group {
                name,
                selector,
                members,
                title,
                description,
            } => {
                validate_tool_name(name, "native tool")?;
                validate_tool_name(selector, "native group selector")?;
                if !tool_names.insert(name.clone()) {
                    return Err(build_error(format!(
                        "native surface declares duplicate tool name `{name}`"
                    )));
                }
                if members.len() < 2 {
                    return Err(build_error(format!(
                        "native group `{name}` must contain at least two members"
                    )));
                }
                let mut selector_values = BTreeSet::new();
                let mut member_operations = Vec::new();
                let mut member_commands = Vec::new();
                let mut route_members = BTreeMap::new();
                for member in members {
                    validate_tool_name(&member.selector_value, "native group selector value")?;
                    if !selector_values.insert(member.selector_value.clone()) {
                        return Err(build_error(format!(
                            "native group `{name}` declares duplicate selector value `{}`",
                            member.selector_value
                        )));
                    }
                    let operation = operation_specs.get(&member.operation_id).ok_or_else(|| {
                        build_error(format!(
                            "native group `{name}` references unknown operation `{}`",
                            member.operation_id
                        ))
                    })?;
                    let command = command_specs.get(&member.operation_id).ok_or_else(|| {
                        build_error(format!(
                            "native group `{name}` cannot resolve command `{}`",
                            member.operation_id
                        ))
                    })?;
                    if !mapped_operations.insert(member.operation_id.clone()) {
                        return Err(build_error(format!(
                            "operation `{}` is mapped more than once",
                            member.operation_id
                        )));
                    }
                    if command.arg(selector).is_some() {
                        return Err(build_error(format!(
                            "native group `{name}` selector `{selector}` collides with operation `{}`",
                            member.operation_id
                        )));
                    }
                    validate_application_error_dialect(
                        &declaration.application_errors,
                        operation,
                        &declaration,
                    )?;
                    member_operations.push((*operation).clone());
                    member_commands.push((*command, member));
                    route_members
                        .insert(member.selector_value.clone(), member.operation_id.clone());
                }
                let task_support = member_operations[0].task_support.clone();
                if member_operations
                    .iter()
                    .any(|operation| operation.task_support != task_support)
                {
                    return Err(build_error(format!(
                        "native group `{name}` contains mixed task support"
                    )));
                }
                let mut input = compile_group_input(registry, name, selector, &member_commands)?;
                for (command, _) in &member_commands {
                    refine_input_schema_for_bindings(
                        &mut input,
                        command,
                        registry,
                        &effective_bindings,
                    );
                }
                let output = compile_group_output(name, selector, &member_operations, members)?;
                let display_title = title.clone().unwrap_or_else(|| name.clone());
                let final_description = description
                    .clone()
                    .unwrap_or_else(|| format!("Select one operation with `{selector}`."));
                let final_description = append_group_guidance(
                    final_description,
                    &member_operations,
                    members,
                    &operation_specs,
                    &declared_routes,
                );
                let final_description = append_resource_binding_guidance(
                    final_description,
                    member_commands.iter().map(|(command, _)| *command),
                    registry,
                    &effective_bindings,
                );
                let defaults = presentation_defaults(&display_title)?;
                tools.push(
                    Tool::new(name.clone(), final_description, input)
                        .with_title(display_title.clone())
                        .with_raw_output_schema(Arc::new(output))
                        .with_execution(
                            ToolExecution::new().with_task_support(TaskSupport::Forbidden),
                        )
                        .annotate(annotations_for_operations(
                            &member_operations,
                            &display_title,
                        )),
                );
                for (operation, member) in member_operations.iter().zip(members) {
                    operations.push(NativeSurfaceOperation {
                        spec: operation.clone(),
                        call: NativeSurfaceCall {
                            tool: name.clone(),
                            arguments: Some(BTreeMap::from([(
                                selector.clone(),
                                Value::String(member.selector_value.clone()),
                            )])),
                        },
                        presentation_defaults: defaults.clone(),
                    });
                }
                routes.insert(
                    name.clone(),
                    CompiledNativeRoute::Group {
                        selector: selector.clone(),
                        members: route_members,
                    },
                );
            }
        }
    }

    validate_exposure(
        &declaration.exposure,
        &operation_specs,
        &mapped_operations,
        registry,
    )?;

    let server_instructions = registry
        .preamble()
        .unwrap_or("Call the named tools directly.")
        .to_string();
    let catalog_hash = registry.catalog_identity().catalog_hash;
    let operations_document = operations
        .iter()
        .map(operation_document)
        .collect::<Result<Vec<_>>>()?;
    let document = json!({
        "version": SNAPSHOT_VERSION,
        "protocolVersion": target.as_str(),
        "name": declaration.name,
        "catalogHash": catalog_hash,
        "declaration": declaration,
        "server": { "instructions": server_instructions },
        "tools": tools,
        "operations": operations_document,
    });
    let canonical_json = canonical_json(&document)?;
    let surface_hash = framed_snapshot_hash(NATIVE_HASH_DOMAIN, SNAPSHOT_VERSION, &canonical_json);
    let identity = ServingSurfaceIdentity::new(declaration.name.clone(), surface_hash.clone())?;
    let snapshot = NativeToolSurfaceSnapshot {
        version: SNAPSHOT_VERSION,
        protocol_version: target.as_str().to_string(),
        name: identity.name,
        catalog_hash,
        surface_hash,
        declaration: declaration.clone(),
        server_instructions,
        tools,
        operations,
        document,
        canonical_json: canonical_json.into_boxed_slice(),
    };
    Ok(NativeToolSurface {
        declaration,
        snapshot: Box::new(snapshot),
        routes,
        resource_binders,
    })
}

fn validate_registry(registry: &CommandRegistry) -> Result<()> {
    registry.validate_effects()?;
    registry.validate_guidance()?;
    registry.validate_types()?;
    registry.validate_argument_schemas()?;
    registry.validate_presentations()?;
    registry.validate_workspaces()?;
    registry.validate_capabilities()?;
    registry.validate_resources()?;
    registry.validate_results()?;
    Ok(())
}

fn normalize_resource_bindings(
    declaration: &mut NativeToolSurfaceDecl,
    command_specs: &BTreeMap<String, &crate::CommandSpec>,
    resource_binders: &BTreeMap<String, Arc<dyn crate::ambient_resources::ErasedAmbientBinder>>,
    binder_error_footprints: &BTreeMap<String, Vec<String>>,
    registry: &CommandRegistry,
) -> Result<()> {
    let exposed_operations = declaration
        .tools
        .iter()
        .flat_map(|tool| match tool {
            NativeToolDecl::Direct { operation_id, .. } => vec![operation_id.as_str()],
            NativeToolDecl::Group { members, .. } => members
                .iter()
                .map(|member| member.operation_id.as_str())
                .collect(),
        })
        .collect::<BTreeSet<_>>();
    let mut used_resources = BTreeSet::new();
    for operation_id in &exposed_operations {
        let Some(command) = command_specs.get(*operation_id) else {
            continue;
        };
        used_resources.extend(command.requires_resources.iter().cloned());
        used_resources.extend(command.optional_resources.iter().cloned());
        used_resources.extend(command.releases.iter().cloned());
    }

    let mut authored = BTreeMap::new();
    for binding in std::mem::take(&mut declaration.resource_bindings) {
        if authored.insert(binding.resource.clone(), binding).is_some() {
            return Err(build_error(
                "native surface declares a resource binding more than once",
            ));
        }
    }
    for resource in authored.keys() {
        if registry.resource_decl(resource).is_none() {
            return Err(build_error(format!(
                "native surface binds unknown resource `{resource}`"
            )));
        }
        if !used_resources.contains(resource) {
            return Err(build_error(format!(
                "native surface binds unused resource `{resource}`"
            )));
        }
    }
    for resource in used_resources {
        authored
            .entry(resource.clone())
            .or_insert(crate::ResourceBindingDecl {
                resource,
                mode: crate::ResourceBindingMode::Argument,
            });
    }

    for (resource, binding) in &authored {
        match &binding.mode {
            crate::ResourceBindingMode::Argument => {
                if resource_binders.contains_key(resource) {
                    return Err(build_error(format!(
                        "argument-bound resource `{resource}` cannot have an ambient binder"
                    )));
                }
            }
            crate::ResourceBindingMode::Ambient { missing_error, .. } => {
                if !resource_binders.contains_key(resource) {
                    return Err(build_error(format!(
                        "ambient resource `{resource}` has no attached binder"
                    )));
                }
                let codes = binder_error_footprints
                    .get(resource)
                    .map(Vec::as_slice)
                    .unwrap_or_default();
                let consumers = exposed_operations
                    .iter()
                    .filter_map(|operation_id| command_specs.get(*operation_id).copied())
                    .filter(|command| {
                        command.requires_resources.contains(resource)
                            || command.optional_resources.contains(resource)
                            || command.releases.contains(resource)
                    })
                    .collect::<Vec<_>>();
                for command in &consumers {
                    let declared = command
                        .output
                        .as_ref()
                        .and_then(|output| output.application.as_ref())
                        .map(|contract| {
                            contract
                                .errors
                                .iter()
                                .map(|error| error.code.as_str())
                                .collect::<BTreeSet<_>>()
                        })
                        .unwrap_or_default();
                    if let Some(code) = codes.iter().find(|code| !declared.contains(code.as_str()))
                    {
                        return Err(build_error(format!(
                            "ambient binder for resource `{resource}` may emit application error `{code}`, which command `{}` does not declare",
                            command.name()
                        )));
                    }
                }
                if let Some(code) = missing_error {
                    let required = consumers
                        .iter()
                        .filter(|command| {
                            command.requires_resources.contains(resource)
                                || command.releases.contains(resource)
                        })
                        .copied()
                        .collect::<Vec<_>>();
                    if required.is_empty() {
                        return Err(build_error(format!(
                            "ambient resource `{resource}` declares dead missing error `{code}`"
                        )));
                    }
                    for command in required {
                        let contract = command
                            .output
                            .as_ref()
                            .and_then(|output| output.application.as_ref())
                            .ok_or_else(|| {
                                build_error(format!(
                                    "ambient missing error `{code}` requires command `{}` to use an application result contract",
                                    command.name()
                                ))
                            })?;
                        crate::results::validate_application_error(
                            contract,
                            crate::results::RawApplicationError {
                                code: code.clone(),
                                message: None,
                                details: json!({}),
                                recovery: crate::ApplicationRecoverySelection::Declared,
                            },
                        )
                        .map_err(|_| {
                            build_error(format!(
                                "ambient missing error `{code}` is not a static declaration-summary error with empty details and valid declared recovery on command `{}`",
                                command.name()
                            ))
                        })?;
                    }
                }
            }
        }
    }
    for resource in resource_binders.keys() {
        if !authored.contains_key(resource) {
            return Err(build_error(format!(
                "native surface attaches binder for unused resource `{resource}`"
            )));
        }
    }

    declaration.resource_bindings = authored.into_values().collect();
    Ok(())
}

fn refine_input_schema_for_bindings(
    schema: &mut JsonObject,
    command: &crate::CommandSpec,
    registry: &CommandRegistry,
    bindings: &BTreeMap<String, crate::ResourceBindingMode>,
) {
    for resource in command
        .requires_resources
        .iter()
        .chain(&command.optional_resources)
        .chain(&command.releases)
    {
        let Some(mode) = bindings.get(resource) else {
            continue;
        };
        let Some(declaration) = registry.resource_decl(resource) else {
            continue;
        };
        let carrier = declaration.carrier_name();
        match mode {
            crate::ResourceBindingMode::Argument => {}
            crate::ResourceBindingMode::Ambient {
                explicit: crate::ExplicitCarrierPolicy::OptionalOverride,
                ..
            } => refine_carrier(schema, &carrier, false),
            crate::ResourceBindingMode::Ambient {
                explicit: crate::ExplicitCarrierPolicy::Omitted,
                ..
            } => refine_carrier(schema, &carrier, true),
        }
    }
}

fn append_resource_binding_guidance<'a>(
    mut description: String,
    commands: impl IntoIterator<Item = &'a crate::CommandSpec>,
    registry: &CommandRegistry,
    bindings: &BTreeMap<String, crate::ResourceBindingMode>,
) -> String {
    let resources = commands
        .into_iter()
        .flat_map(|command| {
            command
                .requires_resources
                .iter()
                .chain(&command.optional_resources)
                .chain(&command.releases)
        })
        .collect::<BTreeSet<_>>();
    if resources.is_empty() {
        return description;
    }

    description.push_str("\n\nResource bindings:");
    for resource in resources {
        let Some(mode) = bindings.get(resource) else {
            continue;
        };
        let carrier = registry
            .resource_decl(resource)
            .map(crate::ResourceDecl::carrier_name)
            .unwrap_or_else(|| format!("{resource}_id"));
        match mode {
            crate::ResourceBindingMode::Argument => description.push_str(&format!(
                "\n- `{resource}` supplied by argument `{carrier}`."
            )),
            crate::ResourceBindingMode::Ambient {
                explicit: crate::ExplicitCarrierPolicy::Omitted,
                ..
            } => description.push_str(&format!("\n- `{resource}` supplied by host.")),
            crate::ResourceBindingMode::Ambient {
                explicit: crate::ExplicitCarrierPolicy::OptionalOverride,
                ..
            } => description.push_str(&format!(
                "\n- `{resource}` supplied by host; explicit override `{carrier}` accepted."
            )),
        }
    }
    description
}

fn refine_carrier(schema: &mut JsonObject, carrier: &str, omit: bool) {
    fn visit(value: &mut Value, carrier: &str, omit: bool) {
        let Some(object) = value.as_object_mut() else {
            return;
        };
        if let Some(required) = object.get_mut("required").and_then(Value::as_array_mut) {
            required.retain(|name| name.as_str() != Some(carrier));
        }
        if omit
            && let Some(properties) = object.get_mut("properties").and_then(Value::as_object_mut)
        {
            properties.remove(carrier);
        }
        for child in object.values_mut() {
            match child {
                Value::Array(values) => {
                    for value in values {
                        visit(value, carrier, omit);
                    }
                }
                Value::Object(_) => visit(child, carrier, omit),
                _ => {}
            }
        }
    }
    let mut value = Value::Object(std::mem::take(schema));
    visit(&mut value, carrier, omit);
    let Value::Object(refined) = value else {
        unreachable!("native input schema remains an object")
    };
    *schema = refined;
}

fn validate_surface_name(name: &str) -> Result<()> {
    ServingSurfaceIdentity::new(name, "0".repeat(64)).map(|_| ())
}

fn validate_tool_name(name: &str, location: &str) -> Result<()> {
    if name.is_empty()
        || name.len() > 128
        || !name.is_ascii()
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b'-'))
    {
        return Err(build_error(format!(
            "{location} must use 1-128 portable ASCII name characters"
        )));
    }
    Ok(())
}

fn schema_object(value: Value, location: &str) -> Result<JsonObject> {
    value
        .as_object()
        .cloned()
        .ok_or_else(|| build_error(format!("{location} must be an object")))
}

fn application_success_schema(operation: &OperationSpec) -> Result<&Value> {
    operation
        .output
        .application
        .as_ref()
        .map(|contract| &contract.success_schema)
        .ok_or_else(|| {
            build_error(format!(
                "native operation `{}` must use an RFC 0014 application result contract",
                operation.id
            ))
        })
}

fn presentation_defaults(title: &str) -> Result<SurfacePresentationDefaults> {
    SurfacePresentationDefaults::new(
        format!("Running {title}"),
        "Confirmation required",
        format!("Run {title}?"),
    )
}

fn annotations_for_operations(operations: &[OperationSpec], title: &str) -> ToolAnnotations {
    let read_only = operations
        .iter()
        .all(|operation| !effect_is_mutating(&operation.effect));
    let destructive = operations.iter().any(|operation| {
        effect_contains(&operation.effect, &crate::EffectSpec::Delete)
            || effect_contains(&operation.effect, &crate::EffectSpec::Exec)
    });
    let idempotent = operations.iter().all(|operation| operation.idempotent);
    let open_world = operations.iter().any(|operation| {
        effect_contains(&operation.effect, &crate::EffectSpec::Network)
            || effect_contains(&operation.effect, &crate::EffectSpec::Exec)
    });
    ToolAnnotations::with_title(title)
        .read_only(read_only)
        .destructive(destructive)
        .idempotent(idempotent)
        .open_world(open_world)
}

fn effect_is_mutating(effect: &crate::EffectSpec) -> bool {
    match effect {
        crate::EffectSpec::Write
        | crate::EffectSpec::Delete
        | crate::EffectSpec::Exec
        | crate::EffectSpec::Custom(_) => true,
        crate::EffectSpec::Composite(parts) => parts.iter().any(effect_is_mutating),
        crate::EffectSpec::Pure | crate::EffectSpec::Read | crate::EffectSpec::Network => false,
    }
}

fn effect_contains(effect: &crate::EffectSpec, needle: &crate::EffectSpec) -> bool {
    effect == needle
        || matches!(effect, crate::EffectSpec::Composite(parts) if parts.iter().any(|part| effect_contains(part, needle)))
}

fn append_group_guidance(
    mut description: String,
    operations: &[OperationSpec],
    members: &[NativeToolMember],
    operation_specs: &BTreeMap<String, OperationSpec>,
    routes: &NativeRouteProjection,
) -> String {
    description.push_str("\n\nOperations:");
    for (operation, member) in operations.iter().zip(members) {
        description.push_str(&format!(
            "\n- `{}`: {}",
            member.selector_value, operation.summary
        ));
        if let Some(use_when) = &operation.use_when {
            description.push_str(&format!(" — use when {use_when}"));
        }
        append_guidance_edges(&mut description, operation, operation_specs, routes, "  ");
    }
    description
}

fn append_operation_guidance(
    mut description: String,
    operation: &OperationSpec,
    operation_specs: &BTreeMap<String, OperationSpec>,
    routes: &NativeRouteProjection,
) -> String {
    if let Some(use_when) = &operation.use_when {
        description.push_str(&format!("\n\nUse when: {use_when}"));
    }
    append_guidance_edges(&mut description, operation, operation_specs, routes, "");
    description
}

fn append_guidance_edges(
    description: &mut String,
    operation: &OperationSpec,
    operation_specs: &BTreeMap<String, OperationSpec>,
    routes: &NativeRouteProjection,
    prefix: &str,
) {
    let by_name = operation_specs
        .values()
        .map(|candidate| (candidate.name(), candidate.id.as_str()))
        .collect::<BTreeMap<_, _>>();
    for alternative in &operation.alternatives {
        if let Some(target) = by_name
            .get(&alternative.command)
            .and_then(|id| routes.get(*id))
        {
            description.push_str(&format!(
                "\n{prefix}Use instead: {} — {}",
                native_call_label(target),
                alternative.when
            ));
        }
    }
    if let Some(fallback) = &operation.fallback {
        let preferred = fallback
            .prefer
            .iter()
            .filter_map(|name| by_name.get(name))
            .filter_map(|id| routes.get(*id))
            .map(native_call_label)
            .collect::<Vec<_>>();
        if !preferred.is_empty() {
            description.push_str(&format!(
                "\n{prefix}Fallback: prefer {} — {}",
                preferred.join(", "),
                fallback.when
            ));
        }
    }
    let mut reverse = operation_specs
        .values()
        .filter(|candidate| {
            candidate.fallback.as_ref().is_some_and(|fallback| {
                fallback.prefer.iter().any(|name| name == &operation.name())
            })
        })
        .collect::<Vec<_>>();
    reverse.sort_by(|left, right| left.id.cmp(&right.id));
    for candidate in reverse {
        if let (Some(route), Some(fallback)) = (routes.get(&candidate.id), &candidate.fallback) {
            description.push_str(&format!(
                "\n{prefix}Fallback for: {} — {}",
                native_call_label(route),
                fallback.when
            ));
        }
    }
}

fn native_call_label(route: &(String, Option<(String, String)>)) -> String {
    match &route.1 {
        Some((selector, value)) => format!("`{}` with `{}={}`", route.0, selector, value),
        None => format!("`{}`", route.0),
    }
}

fn compile_group_input(
    registry: &CommandRegistry,
    group: &str,
    selector: &str,
    members: &[(&crate::CommandSpec, &NativeToolMember)],
) -> Result<JsonObject> {
    let mut properties = Map::new();
    properties.insert(
        selector.to_string(),
        json!({
            "type": "string",
            "enum": members
                .iter()
                .map(|(_, member)| member.selector_value.clone())
                .collect::<Vec<_>>()
        }),
    );
    let mut definitions = Map::new();
    let mut required_intersection: Option<BTreeSet<String>> = None;
    let mut member_properties = Vec::new();

    for (command, member) in members {
        let schema = registry.arg_schema(command);
        let object = schema.as_object().ok_or_else(|| {
            build_error(format!(
                "native group `{group}` member `{}` has non-object input schema",
                member.operation_id
            ))
        })?;
        let own_properties = object
            .get("properties")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        for (name, schema) in &own_properties {
            match properties.get(name) {
                Some(existing) if existing != schema => {
                    return Err(build_error(format!(
                        "native group `{group}` has incompatible schema for shared argument `{name}`"
                    )));
                }
                Some(_) => {}
                None => {
                    properties.insert(name.clone(), schema.clone());
                }
            }
        }
        merge_definitions(group, &mut definitions, object.get("$defs"))?;
        let required = object
            .get("required")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(ToOwned::to_owned)
            .collect::<BTreeSet<_>>();
        required_intersection = Some(match required_intersection.take() {
            Some(current) => current.intersection(&required).cloned().collect(),
            None => required,
        });
        member_properties.push((command, own_properties));
    }

    let mut relationships = BTreeMap::<String, BTreeSet<String>>::new();
    let triggers = members
        .iter()
        .flat_map(|(command, _)| command.args.iter().map(|argument| argument.name.clone()))
        .collect::<BTreeSet<_>>();
    for trigger in triggers {
        let mut common: Option<BTreeSet<String>> = None;
        for (command, own_properties) in &member_properties {
            if !own_properties.contains_key(&trigger) {
                continue;
            }
            let targets = command
                .arg(&trigger)
                .map(|argument| argument.requires_arguments.iter().cloned().collect())
                .unwrap_or_default();
            if let Some(existing) = &common {
                if existing != &targets {
                    return Err(build_error(format!(
                        "native group `{group}` has incompatible presence relationships for `{trigger}`"
                    )));
                }
            } else {
                common = Some(targets);
            }
        }
        if let Some(targets) = common
            && !targets.is_empty()
        {
            for target in &targets {
                if !properties.contains_key(target) {
                    return Err(build_error(format!(
                        "native group `{group}` presence relationship `{trigger}` -> `{target}` is absent from the property union"
                    )));
                }
            }
            relationships.insert(trigger, targets);
        }
    }

    let mut required = vec![Value::String(selector.to_string())];
    required.extend(
        required_intersection
            .unwrap_or_default()
            .into_iter()
            .map(Value::String),
    );
    let mut schema = Map::from_iter([
        ("type".to_string(), Value::String("object".to_string())),
        ("properties".to_string(), Value::Object(properties)),
        ("required".to_string(), Value::Array(required)),
        ("additionalProperties".to_string(), Value::Bool(false)),
    ]);
    if !definitions.is_empty() {
        schema.insert("$defs".to_string(), Value::Object(definitions));
    }
    if !relationships.is_empty() {
        schema.insert(
            "dependencies".to_string(),
            Value::Object(
                relationships
                    .into_iter()
                    .map(|(trigger, targets)| {
                        (
                            trigger,
                            Value::Array(targets.into_iter().map(Value::String).collect()),
                        )
                    })
                    .collect(),
            ),
        );
    }
    Ok(schema)
}

fn merge_definitions(
    group: &str,
    target: &mut Map<String, Value>,
    value: Option<&Value>,
) -> Result<()> {
    let Some(definitions) = value.and_then(Value::as_object) else {
        return Ok(());
    };
    for (name, schema) in definitions {
        match target.get(name) {
            Some(existing) if existing != schema => {
                return Err(build_error(format!(
                    "native group `{group}` has conflicting `$defs` entry `{name}`"
                )));
            }
            Some(_) => {}
            None => {
                target.insert(name.clone(), schema.clone());
            }
        }
    }
    Ok(())
}

fn compile_group_output(
    group: &str,
    selector: &str,
    operations: &[OperationSpec],
    members: &[NativeToolMember],
) -> Result<JsonObject> {
    enum OutputClass {
        Object {
            shape: Value,
            selectors: Vec<String>,
        },
        MemberUnion(Value),
    }

    let mut classes = Vec::<OutputClass>::new();
    let mut definitions = Map::new();
    for (operation, member) in operations.iter().zip(members) {
        let source = application_success_schema(operation)?;
        let mut shape = resolve_group_output_schema(source, source)?;
        if !schema_accepts_objects_only(&shape, &shape)? {
            return Err(build_error(format!(
                "native group `{group}` member `{}` requires an object-only success schema",
                operation.id
            )));
        }
        let object = shape.as_object_mut().ok_or_else(|| {
            build_error(format!(
                "native group `{group}` member `{}` has non-object success schema",
                operation.id
            ))
        })?;
        let member_definitions = object.remove("$defs");
        merge_definitions(group, &mut definitions, member_definitions.as_ref())?;
        if object.contains_key("oneOf") {
            inject_member_union_selector(
                group,
                &operation.id,
                &mut shape,
                source,
                selector,
                &member.selector_value,
            )?;
            classes.push(OutputClass::MemberUnion(shape));
        } else {
            remove_matching_selector(
                group,
                &operation.id,
                &mut shape,
                source,
                selector,
                &member.selector_value,
            )?;
            if let Some(OutputClass::Object { selectors, .. }) = classes.iter_mut().find(
                |class| matches!(class, OutputClass::Object { shape: known, .. } if known == &shape),
            ) {
                selectors.push(member.selector_value.clone());
            } else {
                classes.push(OutputClass::Object {
                    shape,
                    selectors: vec![member.selector_value.clone()],
                });
            }
        }
    }

    // The released VBL surface orders singleton shape classes before
    // coalesced classes. This stable sort preserves declaration order within
    // each cardinality and selector order within each class.
    classes.sort_by_key(|class| match class {
        OutputClass::Object { selectors, .. } => selectors.len(),
        OutputClass::MemberUnion(_) => 1,
    });
    let mut branches = Vec::new();
    for class in classes {
        match class {
            OutputClass::Object {
                mut shape,
                selectors,
            } => {
                insert_selector(&mut shape, selector, &selectors)?;
                branches.push(shape);
            }
            OutputClass::MemberUnion(shape) => branches.push(shape),
        }
    }
    let mut output = if branches.len() == 1 {
        branches.pop().expect("one branch")
    } else {
        json!({ "oneOf": branches })
    };
    if !definitions.is_empty() {
        output
            .as_object_mut()
            .expect("group output root is an object")
            .insert("$defs".to_string(), Value::Object(definitions));
    }
    schema_object(output, "group output schema")
}

fn remove_matching_selector(
    group: &str,
    operation_id: &str,
    shape: &mut Value,
    root: &Value,
    selector: &str,
    selector_value: &str,
) -> Result<()> {
    remove_matching_selector_with_ancestor(
        group,
        operation_id,
        shape,
        root,
        selector,
        selector_value,
        false,
    )
}

fn remove_matching_selector_with_ancestor(
    group: &str,
    operation_id: &str,
    shape: &mut Value,
    root: &Value,
    selector: &str,
    selector_value: &str,
    selector_controlled_by_ancestor: bool,
) -> Result<()> {
    let object = shape
        .as_object_mut()
        .ok_or_else(|| build_error("grouped success branch must be an object"))?;
    if let Some(existing) = object
        .get("properties")
        .and_then(Value::as_object)
        .and_then(|properties| properties.get(selector))
    {
        let required = object
            .get("required")
            .and_then(Value::as_array)
            .is_some_and(|required| required.iter().any(|name| name == selector));
        let matches = schema_proves_string_singleton(existing, root, selector_value)?;
        if !required || !matches {
            return Err(build_error(format!(
                "native group `{group}` selector `{selector}` conflicts with operation `{operation_id}` output"
            )));
        }
    } else if object.get("additionalProperties") != Some(&Value::Bool(false))
        && !selector_controlled_by_ancestor
    {
        return Err(build_error(format!(
            "native group `{group}` operation `{operation_id}` must exclude undeclared selector `{selector}` from its output"
        )));
    }
    if let Some(properties) = object.get_mut("properties").and_then(Value::as_object_mut) {
        properties.remove(selector);
    }
    if let Some(required) = object.get_mut("required").and_then(Value::as_array_mut) {
        required.retain(|name| name != selector);
    }
    Ok(())
}

fn schema_proves_string_singleton(schema: &Value, root: &Value, expected: &str) -> Result<bool> {
    let expected = Value::String(expected.to_string());
    if !crate::results::value_matches_schema(&expected, schema, root) {
        return Ok(false);
    }
    let Some(candidates) = finite_schema_candidates(schema, root)? else {
        return Ok(false);
    };
    let mut accepted = Vec::new();
    for candidate in candidates {
        if crate::results::value_matches_schema(&candidate, schema, root)
            && !accepted.contains(&candidate)
        {
            accepted.push(candidate);
        }
    }
    Ok(accepted == [expected])
}

fn finite_schema_candidates(schema: &Value, root: &Value) -> Result<Option<Vec<Value>>> {
    let object = schema
        .as_object()
        .ok_or_else(|| build_error("result schema must be a JSON object"))?;
    if let Some(constant) = object.get("const") {
        return Ok(Some(vec![constant.clone()]));
    }
    if let Some(values) = object.get("enum").and_then(Value::as_array) {
        return Ok(Some(values.clone()));
    }
    if let Some(reference) = object.get("$ref").and_then(Value::as_str)
        && let Some(candidates) =
            finite_schema_candidates(resolve_local_ref(root, reference)?, root)?
    {
        return Ok(Some(candidates));
    }
    if let Some(branches) = object.get("oneOf").and_then(Value::as_array) {
        let mut candidates = Vec::new();
        for branch in branches {
            let Some(branch_candidates) = finite_schema_candidates(branch, root)? else {
                return Ok(None);
            };
            candidates.extend(branch_candidates);
        }
        return Ok(Some(candidates));
    }
    Ok(None)
}

fn insert_selector(shape: &mut Value, selector: &str, selector_values: &[String]) -> Result<()> {
    let object = shape
        .as_object_mut()
        .ok_or_else(|| build_error("grouped success branch must be an object"))?;
    let properties = object
        .entry("properties")
        .or_insert_with(|| Value::Object(Map::new()))
        .as_object_mut()
        .ok_or_else(|| build_error("grouped success `properties` must be an object"))?;
    let selector_schema = if selector_values.len() == 1 {
        json!({ "type": "string", "const": selector_values[0] })
    } else {
        json!({ "type": "string", "enum": selector_values })
    };
    properties.insert(selector.to_string(), selector_schema);
    let required = object
        .entry("required")
        .or_insert_with(|| Value::Array(Vec::new()))
        .as_array_mut()
        .ok_or_else(|| build_error("grouped success `required` must be an array"))?;
    if !required.iter().any(|name| name == selector) {
        required.insert(0, Value::String(selector.to_string()));
    }
    Ok(())
}

fn inject_member_union_selector(
    group: &str,
    operation_id: &str,
    schema: &mut Value,
    root: &Value,
    selector: &str,
    selector_value: &str,
) -> Result<()> {
    inject_member_union_selector_with_ancestor(
        group,
        operation_id,
        schema,
        root,
        selector,
        selector_value,
        false,
    )
}

fn inject_member_union_selector_with_ancestor(
    group: &str,
    operation_id: &str,
    schema: &mut Value,
    root: &Value,
    selector: &str,
    selector_value: &str,
    selector_controlled_by_ancestor: bool,
) -> Result<()> {
    let object = schema
        .as_object_mut()
        .ok_or_else(|| build_error("grouped success union branch must be an object"))?;
    if let Some(reference) = object.get("$ref").and_then(Value::as_str) {
        let mut resolved = resolve_group_output_schema(resolve_local_ref(root, reference)?, root)?;
        for (key, value) in object.iter() {
            if key != "$ref" {
                resolved
                    .as_object_mut()
                    .ok_or_else(|| build_error("grouped `$ref` must resolve to an object"))?
                    .insert(key.clone(), value.clone());
            }
        }
        *schema = resolved;
        return inject_member_union_selector_with_ancestor(
            group,
            operation_id,
            schema,
            root,
            selector,
            selector_value,
            selector_controlled_by_ancestor,
        );
    }
    let selector_controlled_here = object
        .get("properties")
        .and_then(Value::as_object)
        .is_some_and(|properties| properties.contains_key(selector))
        || object
            .get("required")
            .and_then(Value::as_array)
            .is_some_and(|required| required.iter().any(|name| name == selector))
        || object.get("additionalProperties") == Some(&Value::Bool(false));
    if let Some(branches) = object.get_mut("oneOf").and_then(Value::as_array_mut) {
        if branches.is_empty() {
            return Err(build_error(format!(
                "native group `{group}` operation `{operation_id}` has an empty output union"
            )));
        }
        for branch in branches {
            inject_member_union_selector_with_ancestor(
                group,
                operation_id,
                branch,
                root,
                selector,
                selector_value,
                selector_controlled_by_ancestor || selector_controlled_here,
            )?;
        }
        reconcile_union_outer_selector(
            group,
            operation_id,
            schema,
            root,
            selector,
            selector_value,
        )?;
        return Ok(());
    }
    remove_matching_selector_with_ancestor(
        group,
        operation_id,
        schema,
        root,
        selector,
        selector_value,
        selector_controlled_by_ancestor,
    )?;
    insert_selector(schema, selector, &[selector_value.to_string()])
}

fn reconcile_union_outer_selector(
    group: &str,
    operation_id: &str,
    schema: &mut Value,
    root: &Value,
    selector: &str,
    selector_value: &str,
) -> Result<()> {
    let object = schema
        .as_object_mut()
        .ok_or_else(|| build_error("grouped success union root must be an object"))?;
    let has_selector_property = object
        .get("properties")
        .and_then(Value::as_object)
        .is_some_and(|properties| properties.contains_key(selector));
    let requires_selector = object
        .get("required")
        .and_then(Value::as_array)
        .is_some_and(|required| required.iter().any(|name| name == selector));
    let closes_properties = object.get("additionalProperties") == Some(&Value::Bool(false));

    if has_selector_property {
        let existing = object
            .get("properties")
            .and_then(Value::as_object)
            .and_then(|properties| properties.get(selector))
            .expect("selector property was observed");
        if !requires_selector || !schema_proves_string_singleton(existing, root, selector_value)? {
            return Err(build_error(format!(
                "native group `{group}` selector `{selector}` conflicts with operation `{operation_id}` output"
            )));
        }
    }

    if has_selector_property || requires_selector || closes_properties {
        if let Some(properties) = object.get_mut("properties").and_then(Value::as_object_mut) {
            properties.remove(selector);
        }
        if let Some(required) = object.get_mut("required").and_then(Value::as_array_mut) {
            required.retain(|name| name != selector);
        }
        insert_selector(schema, selector, &[selector_value.to_string()])?;
    }
    Ok(())
}

fn resolve_group_output_schema(schema: &Value, root: &Value) -> Result<Value> {
    let object = schema
        .as_object()
        .ok_or_else(|| build_error("grouped success schema must be an object"))?;
    let Some(reference) = object.get("$ref").and_then(Value::as_str) else {
        return Ok(schema.clone());
    };
    let target = resolve_local_ref(root, reference)?;
    let mut resolved = resolve_group_output_schema(target, root)?;
    let resolved_object = resolved
        .as_object_mut()
        .ok_or_else(|| build_error("grouped `$ref` must resolve to an object schema"))?;
    for (key, value) in object {
        if key == "$ref" || key == "$defs" {
            continue;
        }
        if let Some(existing) = resolved_object.get(key)
            && existing != value
        {
            return Err(build_error(format!(
                "grouped root `$ref` has conflicting sibling `{key}`"
            )));
        }
        resolved_object.insert(key.clone(), value.clone());
    }
    if let Some(definitions) = object.get("$defs") {
        resolved_object.insert("$defs".to_string(), definitions.clone());
    }
    Ok(resolved)
}

fn schema_accepts_objects_only(schema: &Value, root: &Value) -> Result<bool> {
    let object = schema
        .as_object()
        .ok_or_else(|| build_error("result schema must be a JSON object"))?;
    if let Some(reference) = object.get("$ref").and_then(Value::as_str)
        && !schema_accepts_objects_only(resolve_local_ref(root, reference)?, root)?
    {
        return Ok(false);
    }
    if let Some(branches) = object.get("oneOf").and_then(Value::as_array) {
        if branches.is_empty() {
            return Ok(false);
        }
        for branch in branches {
            if !schema_accepts_objects_only(branch, root)? {
                return Ok(false);
            }
        }
    }
    match object.get("type") {
        Some(Value::String(kind)) => Ok(kind == "object"),
        Some(Value::Array(kinds)) => {
            Ok(!kinds.is_empty() && kinds.iter().all(|kind| kind == "object"))
        }
        Some(_) => Ok(false),
        None if object.contains_key("oneOf") || object.contains_key("$ref") => Ok(true),
        None => Ok(false),
    }
}

fn resolve_local_ref<'a>(root: &'a Value, reference: &str) -> Result<&'a Value> {
    let pointer = reference
        .strip_prefix('#')
        .ok_or_else(|| build_error("native surfaces accept only local schema references"))?;
    root.pointer(pointer).ok_or_else(|| {
        build_error(format!(
            "native surface schema contains unresolved reference `{reference}`"
        ))
    })
}

fn validate_exposure(
    exposure: &NativeExposurePolicy,
    operations: &BTreeMap<String, OperationSpec>,
    mapped: &BTreeSet<String>,
    registry: &CommandRegistry,
) -> Result<()> {
    let all = operations.keys().cloned().collect::<BTreeSet<_>>();
    let omitted = match exposure {
        NativeExposurePolicy::Complete => BTreeSet::new(),
        NativeExposurePolicy::ExplicitSubset { omitted_operations } => omitted_operations.clone(),
    };
    if let Some(unknown) = omitted.iter().find(|operation| !all.contains(*operation)) {
        return Err(build_error(format!(
            "native surface omits unknown operation `{unknown}`"
        )));
    }
    if let Some(overlap) = omitted.iter().find(|operation| mapped.contains(*operation)) {
        return Err(build_error(format!(
            "native operation `{overlap}` is both mapped and omitted"
        )));
    }
    let covered = mapped.union(&omitted).cloned().collect::<BTreeSet<_>>();
    if covered != all {
        let missing = all.difference(&covered).cloned().collect::<Vec<_>>();
        return Err(build_error(format!(
            "native surface does not account for operations: {}",
            missing.join(", ")
        )));
    }
    validate_steering_closure(mapped, operations, registry)
}

fn validate_steering_closure(
    exposed: &BTreeSet<String>,
    operations: &BTreeMap<String, OperationSpec>,
    registry: &CommandRegistry,
) -> Result<()> {
    let by_name = operations
        .values()
        .map(|operation| (operation.name(), operation.id.clone()))
        .collect::<BTreeMap<_, _>>();
    for operation_id in exposed {
        let operation = &operations[operation_id];
        let mut required = BTreeSet::new();
        for alternative in &operation.alternatives {
            if let Some(target) = by_name.get(&alternative.command) {
                required.insert(target.clone());
            }
        }
        if let Some(fallback) = &operation.fallback {
            for preferred in &fallback.prefer {
                if let Some(target) = by_name.get(preferred) {
                    required.insert(target.clone());
                }
            }
        }
        if let Some(application) = &operation.output.application {
            for error in &application.errors {
                for recovery in &error.recoveries {
                    if let crate::ApplicationRecoveryDecl::Operation { operation_id } = recovery {
                        required.insert(operation_id.clone());
                    }
                }
            }
        }
        for resource in operation
            .requires_resources
            .iter()
            .chain(&operation.grants)
            .chain(&operation.enumerates)
            .chain(&operation.releases)
        {
            for command in registry
                .resource_granters(resource)
                .into_iter()
                .chain(registry.resource_enumerators(resource))
                .chain(registry.resource_releasers(resource))
            {
                if let Some(target) = by_name.get(&command) {
                    required.insert(target.clone());
                }
            }
        }
        for capability in &operation.requires {
            for command in registry.capability_bootstrap_providers(capability) {
                if let Some(target) = by_name.get(&command) {
                    required.insert(target.clone());
                }
            }
        }
        if let Some(target) = required.iter().find(|target| !exposed.contains(*target)) {
            return Err(build_error(format!(
                "native exposure omits `{target}`, which is reachable from `{operation_id}`"
            )));
        }
    }
    Ok(())
}

fn validate_application_error_dialect(
    dialect: &NativeApplicationErrorDialect,
    operation: &OperationSpec,
    declaration: &NativeToolSurfaceDecl,
) -> Result<()> {
    if matches!(dialect, NativeApplicationErrorDialect::Canonical) {
        return Ok(());
    }
    let Some(application) = &operation.output.application else {
        return Err(build_error(format!(
            "native operation `{}` is missing its application result contract",
            operation.id
        )));
    };
    let routes = declaration_routes(declaration);
    let mut flattened = BTreeMap::<String, String>::new();
    for error in &application.errors {
        if error.details_schema
            != json!({ "type": "object", "properties": {}, "additionalProperties": false })
            && error.details_schema != json!({ "type": "object", "additionalProperties": false })
        {
            return Err(build_error(format!(
                "flat native errors require empty details for `{}`",
                error.code
            )));
        }
        if !matches!(
            error.recovery_cardinality,
            crate::RecoveryCardinality::AtMostOne
        ) {
            return Err(build_error(format!(
                "flat native errors require at-most-one recovery for `{}`",
                error.code
            )));
        }
        for recovery in &error.recoveries {
            let (key, value) = match recovery {
                crate::ApplicationRecoveryDecl::Operation { operation_id } => {
                    let route = routes.get(operation_id).ok_or_else(|| {
                        build_error(format!(
                            "native error recovery `{operation_id}` is not exposed"
                        ))
                    })?;
                    if route.1.is_some() {
                        return Err(build_error(format!(
                            "flat native errors cannot target grouped operation `{operation_id}`"
                        )));
                    }
                    (format!("operation:{operation_id}"), route.0.clone())
                }
                crate::ApplicationRecoveryDecl::Action(action) => {
                    if declaration.tools.iter().any(|tool| match tool {
                        NativeToolDecl::Direct { name, .. }
                        | NativeToolDecl::Group { name, .. } => name == &action.code,
                    }) || matches!(
                        &declaration.framework_help,
                        FrameworkHelpProjection::Tool { name } if name == &action.code
                    ) {
                        return Err(build_error(format!(
                            "flat native recovery token `{}` is ambiguous with an exposed tool",
                            action.code
                        )));
                    }
                    (format!("action:{}", action.code), action.code.clone())
                }
            };
            if let Some(existing) = flattened.insert(value.clone(), key.clone())
                && existing != key
            {
                return Err(build_error(format!(
                    "flat native recovery token `{value}` is ambiguous"
                )));
            }
        }
    }
    Ok(())
}

fn declaration_routes(declaration: &NativeToolSurfaceDecl) -> NativeRouteProjection {
    let mut routes = BTreeMap::new();
    for tool in &declaration.tools {
        match tool {
            NativeToolDecl::Direct {
                name, operation_id, ..
            } => {
                routes.insert(operation_id.clone(), (name.clone(), None));
            }
            NativeToolDecl::Group {
                name,
                selector,
                members,
                ..
            } => {
                for member in members {
                    routes.insert(
                        member.operation_id.clone(),
                        (
                            name.clone(),
                            Some((selector.clone(), member.selector_value.clone())),
                        ),
                    );
                }
            }
        }
    }
    routes
}

fn operation_document(operation: &NativeSurfaceOperation) -> Result<Value> {
    let mut call = Map::new();
    call.insert(
        "tool".to_string(),
        Value::String(operation.call.tool.clone()),
    );
    if let Some(arguments) = &operation.call.arguments {
        call.insert(
            "arguments".to_string(),
            serde_json::to_value(arguments)
                .map_err(|error| build_error(format!("cannot serialize surface call: {error}")))?,
        );
    }
    Ok(json!({
        "spec": operation.spec,
        "call": call,
        "presentationDefaults": {
            "invocationMessage": operation.presentation_defaults.invocation_message(),
            "confirmationTitle": operation.presentation_defaults.confirmation_title(),
            "confirmationMessage": operation.presentation_defaults.confirmation_message(),
        }
    }))
}

pub(crate) fn compile_effect_lane_surface(
    registry: &CommandRegistry,
    tools: &[Tool],
    instructions: &str,
    presentation_defaults: &BTreeMap<String, SurfacePresentationDefaults>,
) -> Result<EffectLaneSurface> {
    let document = json!({
        "version": SNAPSHOT_VERSION,
        "protocolVersion": "2025-11-25",
        "name": "effect-lanes",
        "catalogHash": registry.catalog_identity().catalog_hash,
        "server": { "instructions": instructions },
        "tools": tools,
        "presentationDefaults": presentation_defaults.iter().map(|(tool, defaults)| {
            (tool.clone(), json!({
                "invocationMessage": defaults.invocation_message(),
                "confirmationTitle": defaults.confirmation_title(),
                "confirmationMessage": defaults.confirmation_message(),
            }))
        }).collect::<Map<String, Value>>(),
    });
    let canonical_json = canonical_json(&document)?;
    let hash = framed_snapshot_hash(EFFECT_LANE_HASH_DOMAIN, SNAPSHOT_VERSION, &canonical_json);
    Ok(EffectLaneSurface {
        snapshot: EffectLaneSurfaceSnapshot {
            identity: ServingSurfaceIdentity::new("effect-lanes", hash)?,
            document,
            tools: tools.to_vec(),
            instructions: instructions.to_string(),
        },
    })
}

impl EffectLaneSurface {
    pub(crate) fn identity(&self) -> &ServingSurfaceIdentity {
        &self.snapshot.identity
    }

    pub(crate) fn document(&self) -> &Value {
        &self.snapshot.document
    }

    pub(crate) fn tools(&self) -> &[Tool] {
        &self.snapshot.tools
    }

    pub(crate) fn instructions(&self) -> &str {
        &self.snapshot.instructions
    }
}

fn framed_snapshot_hash(domain: &str, version: u32, payload: &[u8]) -> String {
    let mut framed = Vec::with_capacity(domain.len() + payload.len() + 13);
    framed.extend_from_slice(domain.as_bytes());
    framed.push(0);
    framed.extend_from_slice(&version.to_be_bytes());
    framed.extend_from_slice(&(payload.len() as u64).to_be_bytes());
    framed.extend_from_slice(payload);
    Sha256::digest(framed)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn canonical_json(value: &Value) -> Result<Vec<u8>> {
    validate_ijson(value)?;
    let mut output = Vec::new();
    write_canonical_json(value, &mut output)?;
    Ok(output)
}

fn validate_ijson(value: &Value) -> Result<()> {
    match value {
        Value::Number(number) => {
            const MAX_EXACT_INTEGER: u64 = 9_007_199_254_740_991;
            if number
                .as_u64()
                .is_some_and(|number| number > MAX_EXACT_INTEGER)
                || number
                    .as_i64()
                    .is_some_and(|number| number.unsigned_abs() > MAX_EXACT_INTEGER)
                || number.as_f64().is_some_and(|number| !number.is_finite())
            {
                return Err(build_error(
                    "native surface number is outside the exact I-JSON domain",
                ));
            }
        }
        Value::Array(values) => {
            for value in values {
                validate_ijson(value)?;
            }
        }
        Value::Object(values) => {
            for value in values.values() {
                validate_ijson(value)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn write_canonical_json(value: &Value, output: &mut Vec<u8>) -> Result<()> {
    match value {
        Value::Null => output.extend_from_slice(b"null"),
        Value::Bool(true) => output.extend_from_slice(b"true"),
        Value::Bool(false) => output.extend_from_slice(b"false"),
        Value::Number(number) => {
            let spelling = canonical_number(number)?;
            output.extend_from_slice(spelling.as_bytes());
        }
        Value::String(string) => output.extend_from_slice(
            serde_json::to_string(string)
                .map_err(|error| build_error(format!("cannot encode canonical string: {error}")))?
                .as_bytes(),
        ),
        Value::Array(values) => {
            output.push(b'[');
            for (index, value) in values.iter().enumerate() {
                if index > 0 {
                    output.push(b',');
                }
                write_canonical_json(value, output)?;
            }
            output.push(b']');
        }
        Value::Object(values) => {
            output.push(b'{');
            let mut entries = values.iter().collect::<Vec<_>>();
            entries.sort_by(|left, right| utf16_cmp(left.0, right.0));
            for (index, (key, value)) in entries.into_iter().enumerate() {
                if index > 0 {
                    output.push(b',');
                }
                output.extend_from_slice(
                    serde_json::to_string(key)
                        .map_err(|error| {
                            build_error(format!("cannot encode canonical key: {error}"))
                        })?
                        .as_bytes(),
                );
                output.push(b':');
                write_canonical_json(value, output)?;
            }
            output.push(b'}');
        }
    }
    Ok(())
}

fn canonical_number(number: &serde_json::Number) -> Result<String> {
    if let Some(value) = number.as_u64() {
        return Ok(value.to_string());
    }
    if let Some(value) = number.as_i64() {
        return Ok(value.to_string());
    }
    let value = number
        .to_string()
        .parse::<f64>()
        .ok()
        .filter(|value| value.is_finite())
        .ok_or_else(|| build_error("native surface number is outside the I-JSON domain"))?;
    if value == 0.0 {
        return Ok("0".to_string());
    }
    let magnitude = value.abs();
    let raw = (1..=17)
        .map(|digits| format!("{:.*e}", digits - 1, magnitude))
        .find(|candidate| {
            candidate
                .parse::<f64>()
                .is_ok_and(|parsed| parsed.to_bits() == magnitude.to_bits())
        })
        .ok_or_else(|| build_error("cannot canonicalize JSON number"))?;
    let negative = value.is_sign_negative();
    let unsigned = raw.as_str();
    let Some((mantissa, exponent)) = unsigned.split_once(['e', 'E']) else {
        let mut fixed = unsigned.to_string();
        if fixed.contains('.') {
            while fixed.ends_with('0') {
                fixed.pop();
            }
            if fixed.ends_with('.') {
                fixed.pop();
            }
        }
        return Ok(if negative { format!("-{fixed}") } else { fixed });
    };
    let exponent = exponent
        .parse::<i32>()
        .map_err(|_| build_error("cannot canonicalize JSON number exponent"))?;
    let decimal_index = mantissa.find('.').unwrap_or(mantissa.len());
    let digits = mantissa.replace('.', "");
    let normalized_exponent = exponent + i32::try_from(decimal_index).unwrap_or(i32::MAX) - 1;
    let body = if (-6..=20).contains(&normalized_exponent) {
        let point = normalized_exponent + 1;
        if point <= 0 {
            format!("0.{}{}", "0".repeat((-point) as usize), digits)
        } else if point as usize >= digits.len() {
            format!("{}{}", digits, "0".repeat(point as usize - digits.len()))
        } else {
            let point = point as usize;
            format!("{}.{}", &digits[..point], &digits[point..])
        }
    } else {
        let fraction = if digits.len() == 1 {
            String::new()
        } else {
            format!(".{}", &digits[1..])
        };
        let sign = if normalized_exponent >= 0 { "+" } else { "" };
        format!("{}{fraction}e{sign}{normalized_exponent}", &digits[..1])
    };
    Ok(if negative { format!("-{body}") } else { body })
}

fn utf16_cmp(left: &str, right: &str) -> Ordering {
    left.encode_utf16().cmp(right.encode_utf16())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_json_uses_rfc_8785_number_and_key_spelling() {
        let value: Value = serde_json::from_str(
            r#"{
                "numbers": [333333333.33333329, 1E30, 4.50, 2e-3, 0.000001, 0.0000001, 0.000000000000000000000000001],
                "literals": [null, true, false]
            }"#,
        )
        .unwrap();
        assert_eq!(
            String::from_utf8(canonical_json(&value).unwrap()).unwrap(),
            r#"{"literals":[null,true,false],"numbers":[333333333.3333333,1e+30,4.5,0.002,0.000001,1e-7,1e-27]}"#
        );
    }

    #[test]
    fn canonical_json_rejects_non_interoperable_integers() {
        assert!(canonical_json(&json!(9_007_199_254_740_992_u64)).is_err());
        assert!(canonical_json(&json!(-9_007_199_254_740_992_i64)).is_err());
    }

    #[test]
    fn snapshot_hash_framing_is_domain_separated() {
        let payload = br#"{"a":1}"#;
        assert_eq!(
            framed_snapshot_hash(NATIVE_HASH_DOMAIN, 1, payload),
            "3fdcf5bb2e8bf68d5077504233c9b726ccf7ba839d382796bb0b0f53487472ed"
        );
        assert_ne!(
            framed_snapshot_hash(NATIVE_HASH_DOMAIN, 1, payload),
            framed_snapshot_hash(EFFECT_LANE_HASH_DOMAIN, 1, payload)
        );
        assert_ne!(
            framed_snapshot_hash(NATIVE_HASH_DOMAIN, 1, payload),
            framed_snapshot_hash(NATIVE_HASH_DOMAIN, 2, payload)
        );
    }
}
