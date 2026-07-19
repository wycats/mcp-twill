use std::collections::{BTreeMap, BTreeSet};

use mcp_workspace_resolver::{
    DeclaredWorkspaceRoot, ResolvedWorkspaceRoot, WorkspaceRequirement, WorkspaceSelection,
    normalize_file_uri, path_has_prefix,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{FrameworkError, Result};

fn default_output_format() -> OutputFormat {
    OutputFormat::Structured
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum ArgType {
    String,
    Path,
    Json,
    Bool,
    Number,
    Integer,
    /// References a `TypeDecl` by name; values are matched against the
    /// declared union's variants by the planner.
    Named(String),
    /// A reference to a declared resource (RFC 0012), accepting a bare id
    /// or the resource's full URI. The framework injects arguments of this
    /// type as the carrier for signature-required resources.
    ResourceRef(String),
}

impl ArgType {
    pub fn expected_name(&self) -> &'static str {
        match self {
            ArgType::String => "a string",
            ArgType::Path => "a path string",
            ArgType::Json => "JSON",
            ArgType::Bool => "a boolean",
            ArgType::Number => "a number",
            ArgType::Integer => "an integer",
            ArgType::Named(_) => "a value matching a declared type",
            ArgType::ResourceRef(_) => "a resource reference string",
        }
    }
}

/// One command argument declaration.
///
/// Schema authorship is intentionally split between explicit named and
/// inline constructors. RFC 0008 named union arguments keep `named`; there is
/// no overloaded `schema` constructor whose string/JSON meaning is ambiguous.
///
/// ```compile_fail
/// use mcp_twill::ArgSpec;
///
/// let _ = ArgSpec::schema("value", "named-or-inline", "Value");
/// ```
///
/// ```compile_fail
/// use mcp_twill::arg;
///
/// let _ = arg::schema("value", "named-or-inline");
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ArgSpec {
    pub name: String,
    pub value_type: ArgType,
    pub required: bool,
    pub summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<String>,
    #[serde(default)]
    pub repeated: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<crate::ArgumentSchemaUse>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub requires_arguments: Vec<String>,
}

impl ArgSpec {
    pub fn string(name: impl Into<String>, summary: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            value_type: ArgType::String,
            required: true,
            summary: summary.into(),
            workspace: None,
            repeated: false,
            schema: None,
            requires_arguments: Vec::new(),
        }
    }

    pub fn path(
        name: impl Into<String>,
        summary: impl Into<String>,
        workspace: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            value_type: ArgType::Path,
            required: true,
            summary: summary.into(),
            workspace: Some(workspace.into()),
            repeated: false,
            schema: None,
            requires_arguments: Vec::new(),
        }
    }

    pub fn named(
        name: impl Into<String>,
        type_name: impl Into<String>,
        summary: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            value_type: ArgType::Named(type_name.into()),
            required: true,
            summary: summary.into(),
            workspace: None,
            repeated: false,
            schema: None,
            requires_arguments: Vec::new(),
        }
    }

    pub fn repeated(mut self) -> Self {
        self.repeated = true;
        self
    }

    pub fn integer(name: impl Into<String>, summary: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            value_type: ArgType::Integer,
            required: true,
            summary: summary.into(),
            workspace: None,
            repeated: false,
            schema: None,
            requires_arguments: Vec::new(),
        }
    }

    pub fn enumerated(
        name: impl Into<String>,
        values: impl IntoIterator<Item = impl AsRef<str>>,
        summary: impl Into<String>,
    ) -> Self {
        let summary = summary.into();
        let values = values
            .into_iter()
            .map(|value| Value::String(value.as_ref().to_string()))
            .collect::<Vec<_>>();
        Self {
            name: name.into(),
            value_type: ArgType::String,
            required: true,
            summary: summary.clone(),
            workspace: None,
            repeated: false,
            schema: Some(crate::ArgumentSchemaUse::inline(serde_json::json!({
                "type": "string",
                "enum": values,
                "description": summary,
            }))),
            requires_arguments: Vec::new(),
        }
    }

    pub fn named_schema(
        name: impl Into<String>,
        schema: impl Into<String>,
        summary: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            value_type: ArgType::Json,
            required: true,
            summary: summary.into(),
            workspace: None,
            repeated: false,
            schema: Some(crate::ArgumentSchemaUse::named(schema)),
            requires_arguments: Vec::new(),
        }
    }

    pub fn inline_schema(
        name: impl Into<String>,
        schema: impl Into<serde_json::Value>,
        summary: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            value_type: ArgType::Json,
            required: true,
            summary: summary.into(),
            workspace: None,
            repeated: false,
            schema: Some(crate::ArgumentSchemaUse::inline(schema)),
            requires_arguments: Vec::new(),
        }
    }

    pub fn with_named_schema(mut self, name: impl Into<String>) -> Self {
        self.schema = Some(crate::ArgumentSchemaUse::named(name));
        self
    }

    pub fn with_inline_schema(mut self, schema: impl Into<serde_json::Value>) -> Self {
        self.schema = Some(crate::ArgumentSchemaUse::inline(schema));
        self
    }

    pub fn requires_argument(mut self, name: impl Into<String>) -> Self {
        self.requires_arguments.push(name.into());
        self
    }

    pub fn optional(mut self) -> Self {
        self.required = false;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum PermissionEffect {
    Read,
    Write,
    Delete,
    Exec,
    Network,
    Custom(String),
}

impl PermissionEffect {
    pub fn as_label(&self) -> String {
        match self {
            PermissionEffect::Read => "read".to_string(),
            PermissionEffect::Write => "write".to_string(),
            PermissionEffect::Delete => "delete".to_string(),
            PermissionEffect::Exec => "exec".to_string(),
            PermissionEffect::Network => "network".to_string(),
            PermissionEffect::Custom(value) => value.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PermissionSpec {
    pub effect: PermissionEffect,
    pub scope: String,
    pub description: String,
}

impl PermissionSpec {
    pub fn new(
        effect: PermissionEffect,
        scope: impl Into<String>,
        description: impl Into<String>,
    ) -> Self {
        Self {
            effect,
            scope: scope.into(),
            description: description.into(),
        }
    }

    pub fn read(scope: impl Into<String>, description: impl Into<String>) -> Self {
        Self::new(PermissionEffect::Read, scope, description)
    }

    pub fn write(scope: impl Into<String>, description: impl Into<String>) -> Self {
        Self::new(PermissionEffect::Write, scope, description)
    }

    pub fn delete(scope: impl Into<String>, description: impl Into<String>) -> Self {
        Self::new(PermissionEffect::Delete, scope, description)
    }

    pub fn exec(scope: impl Into<String>, description: impl Into<String>) -> Self {
        Self::new(PermissionEffect::Exec, scope, description)
    }

    pub fn network(scope: impl Into<String>, description: impl Into<String>) -> Self {
        Self::new(PermissionEffect::Network, scope, description)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceDecl {
    pub name: String,
    pub uri: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

impl WorkspaceDecl {
    pub fn new(name: impl Into<String>, uri: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            uri: uri.into(),
            description: None,
        }
    }

    pub fn file(name: impl Into<String>, path: impl Into<String>) -> Self {
        Self::new(name, path)
    }

    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// Reports whether `value` is the workspace root or lies inside it.
    ///
    /// Boundary checks use `mcp-workspace-resolver` path rules: drive-letter
    /// paths compare case-insensitively, POSIX paths case-sensitively, and
    /// non-`file:` URIs never match.
    pub fn contains_path_value(&self, value: &str) -> bool {
        let Ok(root) = normalize_file_uri(&self.uri) else {
            return false;
        };
        let Ok(candidate) = normalize_file_uri(value) else {
            return false;
        };
        path_has_prefix(&candidate, &root)
    }

    /// Projects this declaration into the resolver's declared-root vocabulary.
    pub fn declared_root(&self) -> DeclaredWorkspaceRoot {
        let mut root = DeclaredWorkspaceRoot::new(self.name.clone(), self.uri.clone());
        if let Some(description) = &self.description {
            root = root.with_display_name(description.clone());
        }
        root
    }

    /// Projects this declaration into a resolver workspace requirement: the
    /// requirement id is the declared name and the declared URI becomes the
    /// fallback root. `sole_workspace` grants the single-root convenience
    /// (a lone client root satisfies the requirement without a name match);
    /// it must be true only when this is the server's only declared
    /// workspace, otherwise one client root would satisfy every requirement.
    pub fn requirement(&self, sole_workspace: bool) -> WorkspaceRequirement {
        let selection = if sole_workspace {
            WorkspaceSelection::PrimaryWhenSingleRoot
        } else {
            WorkspaceSelection::ByNameOrAlias
        };
        WorkspaceRequirement::new(self.name.clone())
            .with_selection(selection)
            .with_fallback(self.declared_root())
    }
}

/// A server-level capability declaration: a named precondition that some
/// commands establish and other commands require, with the argument that
/// carries opaque proof of it across calls. The framework validates the
/// declarations and pre-validates call shape; application code validates the
/// proof. Live server-held values use [`ResourceDecl`] instead.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityDecl {
    pub name: String,
    pub summary: String,
    /// The argument name that carries proof of this capability on
    /// commands that require it.
    pub carrier: String,
}

impl CapabilityDecl {
    pub fn new(name: impl Into<String>, summary: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            summary: summary.into(),
            carrier: String::new(),
        }
    }

    /// Names the argument that carries proof of this capability.
    pub fn carried_by(mut self, argument: impl Into<String>) -> Self {
        self.carrier = argument.into();
        self
    }
}

/// A server-held resource with an identity, a lifetime, and commands that
/// mint, enumerate, and release references to it (RFC 0012). Declaring a
/// resource derives a reference argument type (`{name}-ref`) and a
/// capability (`{name}`), so the RFC 0010 vocabulary keeps working; the
/// lifecycle edges themselves derive from handler signatures.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ResourceDecl {
    pub name: String,
    pub summary: String,
    /// URI template with exactly one `{id}` slot, e.g. `vbl://tab/{id}`.
    pub uri: String,
    /// Argument name used when a tier binds this resource's references to
    /// an argument. Defaults to `{name}_id`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub carrier: Option<String>,
    /// Resource this one is scoped within.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub within: Option<String>,
    /// Prose: the window in which references stay valid.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lifetime: Option<String>,
    /// Prose: how the resource leaves the world without an explicit
    /// releasing command (lease expiry, session end).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expiry: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reference_schema: Option<crate::ArgumentSchemaUse>,
}

impl ResourceDecl {
    pub fn new(name: impl Into<String>, summary: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            summary: summary.into(),
            uri: String::new(),
            carrier: None,
            within: None,
            lifetime: None,
            expiry: None,
            reference_schema: None,
        }
    }

    /// The URI template for references to this resource. Must contain
    /// exactly one `{id}` slot.
    pub fn uri(mut self, template: impl Into<String>) -> Self {
        self.uri = template.into();
        self
    }

    /// Names the argument that carries references on argument-bound tiers.
    pub fn carrier(mut self, argument: impl Into<String>) -> Self {
        self.carrier = Some(argument.into());
        self
    }

    /// Scopes this resource inside another declared resource.
    pub fn within(mut self, resource: impl Into<String>) -> Self {
        self.within = Some(resource.into());
        self
    }

    /// Prose describing the window in which references stay valid.
    pub fn lifetime(mut self, prose: impl Into<String>) -> Self {
        self.lifetime = Some(prose.into());
        self
    }

    /// Prose describing how the resource retires without an explicit
    /// releasing command.
    pub fn expiry(mut self, prose: impl Into<String>) -> Self {
        self.expiry = Some(prose.into());
        self
    }

    pub fn reference_schema(mut self, schema: impl Into<crate::ArgumentSchemaUse>) -> Self {
        self.reference_schema = Some(schema.into());
        self
    }

    /// The carrier argument name, defaulted to `{name}_id`.
    pub fn carrier_name(&self) -> String {
        self.carrier
            .clone()
            .unwrap_or_else(|| format!("{}_id", self.name))
    }

    /// The derived reference type name.
    pub fn reference_type_name(&self) -> String {
        format!("{}-ref", self.name)
    }

    pub(crate) fn reference_summary(&self) -> String {
        self.reference_schema
            .as_ref()
            .and_then(|schema| match schema {
                crate::ArgumentSchemaUse::Inline { schema } => schema
                    .get("description")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned),
                crate::ArgumentSchemaUse::Named { .. } => None,
            })
            .unwrap_or_else(|| {
                format!(
                    "The `{}` to operate on; accepts a bare id or its URI.",
                    self.name
                )
            })
    }

    /// The template split at its single `{id}` slot, or `None` when the
    /// template does not have exactly one slot.
    pub fn uri_parts(&self) -> Option<(&str, &str)> {
        let start = self.uri.find("{id}")?;
        let suffix = &self.uri[start + 4..];
        if suffix.contains("{id}") {
            return None;
        }
        Some((&self.uri[..start], suffix))
    }

    /// Mints the URI for a granted id. Refuses ids that would not
    /// round-trip: substitution and parse-back must be exact inverses, so
    /// ids are limited to URI-unreserved characters.
    pub fn mint_uri(&self, id: &str) -> Result<String> {
        if id.is_empty() || !id.bytes().all(uri_unreserved) {
            return Err(FrameworkError::Handler(format!(
                "grant id `{id}` for resource `{}` would not round-trip through `{}`; ids must be non-empty and use URI-unreserved characters (A-Z a-z 0-9 - . _ ~)",
                self.name, self.uri
            )));
        }
        let (prefix, suffix) = self.uri_parts().ok_or_else(|| {
            FrameworkError::Handler(format!(
                "resource `{}` URI template `{}` does not have exactly one `{{id}}` slot",
                self.name, self.uri
            ))
        })?;
        Ok(format!("{prefix}{id}{suffix}"))
    }

    /// Normalizes a reference to its bare id: a full URI matching the
    /// template loses the template text; anything else is already an id.
    pub fn normalize_reference<'a>(&self, reference: &'a str) -> &'a str {
        self.parse_uri(reference).unwrap_or(reference)
    }

    /// Extracts the id from a full URI matching this template, or `None`
    /// when the value is not a URI of this resource. Only ids `mint_uri`
    /// could have produced parse back: without the unreserved-character
    /// check, a broad template like `x://{id}` would claim `x://tab/1`
    /// (id `tab/1`) even though that URI was minted by `x://tab/{id}`.
    pub fn parse_uri<'a>(&self, value: &'a str) -> Option<&'a str> {
        let (prefix, suffix) = self.uri_parts()?;
        let id = value.strip_prefix(prefix)?.strip_suffix(suffix)?;
        (!id.is_empty() && id.bytes().all(uri_unreserved)).then_some(id)
    }
}

fn uri_unreserved(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~')
}

fn all_unreserved(text: &str) -> bool {
    text.bytes().all(uri_unreserved)
}

/// Whether a template starts with an RFC 3986 scheme (`ALPHA *( ALPHA /
/// DIGIT / "+" / "-" / "." ) ":"`). Templates without one mint relative
/// strings, not self-describing URIs.
pub(crate) fn template_has_scheme(template: &str) -> bool {
    let Some(colon) = template.find(':') else {
        return false;
    };
    let mut bytes = template[..colon].bytes();
    match bytes.next() {
        Some(byte) if byte.is_ascii_alphabetic() => {}
        _ => return false,
    }
    bytes.all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'-' | b'.'))
}

/// Whether two templates can mint the same URI from valid (unreserved)
/// ids — like `x://{id}/bar` with id `foo` and `x://foo/{id}` with id
/// `bar`, which both mint `x://foo/bar`. Such a URI would route to
/// whichever declaration matches first, so registration refuses the pair.
pub(crate) fn templates_overlap(a: &ResourceDecl, b: &ResourceDecl) -> bool {
    let (Some(a), Some(b)) = (a.uri_parts(), b.uri_parts()) else {
        return false;
    };
    overlap_directed(a, b) || overlap_directed(b, a)
}

/// Decides whether `p1 · id1 · s1 == p2 · id2 · s2` has a solution with
/// both ids nonempty and unreserved, assuming `p1` is a prefix of `p2`
/// (the only way two prefixes can head the same string). Cases follow
/// from where the end of `id1` falls in the `p2·id2·s2` segmentation.
fn overlap_directed((p1, s1): (&str, &str), (p2, s2): (&str, &str)) -> bool {
    let Some(excess) = p2.strip_prefix(p1) else {
        return false;
    };
    // id1 ends inside s2: s1 is a suffix of s2 and everything consumed by
    // id1 (the prefix excess and the head of s2) must be mintable.
    if let Some(head) = s2.strip_suffix(s1)
        && !head.is_empty()
        && all_unreserved(head)
        && all_unreserved(excess)
    {
        return true;
    }
    // id1 ends inside id2 (or exactly at its end): s1 is s2 plus an
    // unreserved (possibly empty) run that id2 supplies.
    if let Some(mid) = s1.strip_suffix(s2) {
        if all_unreserved(mid) && all_unreserved(excess) {
            return true;
        }
        // id1 ends inside the prefix excess: id1 covers `excess[..cut]`,
        // s1 must continue with the rest of the excess, then a nonempty
        // unreserved run for id2, then s2.
        for cut in 1..excess.len() {
            if !excess.is_char_boundary(cut) {
                continue;
            }
            let (consumed, remaining) = excess.split_at(cut);
            if !all_unreserved(consumed) {
                break;
            }
            if let Some(id2) = mid.strip_prefix(remaining)
                && !id2.is_empty()
                && all_unreserved(id2)
            {
                return true;
            }
        }
    }
    false
}

/// A reference to a declared resource carried in structured output:
/// which resource, the bare id, and the minted URI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ResourceRef {
    pub resource: String,
    pub id: String,
    pub uri: String,
}

/// A workspace root selected for an invocation plan: which root a path
/// argument was planned against, where it came from, and why it was chosen.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PlanWorkspaceRoot {
    pub id: String,
    pub root_uri: String,
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_issuer: Option<String>,
    pub selection_reason: Value,
}

impl PlanWorkspaceRoot {
    /// The root as a filesystem path. Errors when the root URI is not a
    /// `file:` URI, using the resolver's normalization rules. Preserves the
    /// path shape: UNC hosts keep their `//host` prefix, drive-letter paths
    /// keep the drive, and relative paths stay relative.
    pub fn path(&self) -> Result<std::path::PathBuf> {
        let normalized = mcp_workspace_resolver::normalize_file_uri(&self.root_uri)
            .map_err(|err| FrameworkError::Handler(err.to_string()))?;
        let mut path = String::new();
        if let Some(host) = normalized.host() {
            path.push_str("//");
            path.push_str(host);
        }
        if let Some(drive) = normalized.drive() {
            path.push(drive);
            path.push(':');
        }
        let mut first = true;
        for component in normalized.components() {
            if first && !normalized.is_absolute() && normalized.host().is_none() {
                // Relative path: no leading separator.
                path.push_str(component);
            } else {
                path.push('/');
                path.push_str(component);
            }
            first = false;
        }
        if path.is_empty() {
            path.push('/');
        }
        Ok(std::path::PathBuf::from(path))
    }
}

impl From<&ResolvedWorkspaceRoot> for PlanWorkspaceRoot {
    fn from(root: &ResolvedWorkspaceRoot) -> Self {
        let source = serde_json::to_value(root.source)
            .ok()
            .and_then(|value| value.as_str().map(ToOwned::to_owned))
            .unwrap_or_else(|| format!("{:?}", root.source));
        Self {
            id: root.id.as_str().to_string(),
            root_uri: root.root_uri.clone(),
            source,
            source_issuer: root.source_issuer.clone(),
            selection_reason: serde_json::to_value(&root.selection_reason).unwrap_or(Value::Null),
        }
    }
}

/// A directed routing edge to a command serving a neighboring case, with
/// the condition that routes there (RFC 0011). Rendered on the command an
/// agent is about to misuse, not on the target.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Alternative {
    pub command: String,
    pub when: String,
}

/// Marks a command as an escape hatch: the commands to exhaust first and
/// the condition that justifies bypassing them (RFC 0011). Preferred
/// commands render a derived reverse edge; the framework enforces no
/// ordering.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Fallback {
    /// Commands to exhaust first.
    pub prefer: Vec<String>,
    /// The condition that justifies using this command anyway.
    pub when: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CommandExample {
    pub command: String,
    pub summary: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub args: BTreeMap<String, Value>,
}

impl CommandExample {
    pub fn new(command: impl Into<String>, summary: impl Into<String>) -> Self {
        Self {
            command: command.into(),
            summary: summary.into(),
            args: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CommandSpec {
    pub path: Vec<String>,
    pub summary: String,
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub invocation_message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confirmation: Option<crate::ConfirmationPresentation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<crate::OutputContract>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stdin: Option<crate::StdinContract>,
    #[serde(default)]
    pub args: Vec<ArgSpec>,
    #[serde(default)]
    pub permissions: Vec<PermissionSpec>,
    #[serde(default)]
    pub examples: Vec<CommandExample>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub progress: Vec<crate::ProgressPhaseSpec>,
    /// The handler deduplicates re-issued invocations (keyed on the plan's
    /// invocation fingerprint), so an ambiguous failure may be retried.
    /// A declaration the framework trusts, like every other catalog fact.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub idempotent: bool,
    #[serde(default, skip_serializing_if = "crate::TaskSupportSpec::is_optional")]
    pub task_support: crate::TaskSupportSpec,
    /// Workspaces this command requires resolved, beyond those referenced
    /// by path arguments. Names must match server-declared workspaces. The
    /// resolved root is delivered to the handler through the plan; it is
    /// never caller-supplied.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub workspaces: Vec<String>,
    /// Workspaces delivered to the handler when resolution succeeds. An
    /// unresolved optional workspace does not prevent dispatch.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub optional_workspaces: Vec<String>,
    /// Whether the handler can consume optional host-supplied conversation
    /// identity through `CommandContext`.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub uses_conversation_identity: bool,
    /// Capabilities this command requires (names of server-declared
    /// capabilities). The capability's carrier argument must be a required
    /// argument of this command.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub requires: Vec<String>,
    /// Capabilities this command establishes.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub provides: Vec<String>,
    /// One sentence: when this command is the right choice.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub use_when: Option<String>,
    /// Commands serving neighboring cases, with the condition that routes
    /// there.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub alternatives: Vec<Alternative>,
    /// Marks this command as an escape hatch for a preferred path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallback: Option<Fallback>,
    /// Resources this command requires live references to. Derived from
    /// the handler signature (`Res<T>`), never written by authors.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub requires_resources: Vec<String>,
    /// Resources this command may consume when a binding is available.
    /// Derived from `Option<Res<T>>` in the handler signature.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub optional_resources: Vec<String>,
    /// Resources this command grants references to. Derived from the
    /// handler output type (`Granted<T>`), never written by authors.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub grants: Vec<String>,
    /// Resources this command releases. Derived from the handler signature
    /// (`Release<T>`), never written by authors.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub releases: Vec<String>,
    /// Resources this command enumerates. Derived from the handler output
    /// type (`Listed<T>`), never written by authors.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub enumerates: Vec<String>,
}

impl CommandSpec {
    pub fn new(
        path: impl IntoIterator<Item = impl Into<String>>,
        summary: impl Into<String>,
        description: impl Into<String>,
    ) -> Self {
        Self {
            path: path.into_iter().map(Into::into).collect(),
            summary: summary.into(),
            description: description.into(),
            invocation_message: None,
            confirmation: None,
            output: None,
            stdin: None,
            args: Vec::new(),
            permissions: Vec::new(),
            examples: Vec::new(),
            progress: Vec::new(),
            idempotent: false,
            task_support: crate::TaskSupportSpec::Optional,
            workspaces: Vec::new(),
            optional_workspaces: Vec::new(),
            uses_conversation_identity: false,
            requires: Vec::new(),
            provides: Vec::new(),
            use_when: None,
            alternatives: Vec::new(),
            fallback: None,
            requires_resources: Vec::new(),
            optional_resources: Vec::new(),
            grants: Vec::new(),
            releases: Vec::new(),
            enumerates: Vec::new(),
        }
    }

    pub fn name(&self) -> String {
        self.path.join(" ")
    }

    pub fn with_arg(mut self, arg: ArgSpec) -> Self {
        self.args.push(arg);
        self
    }

    pub fn with_output(mut self, output: crate::OutputContract) -> Self {
        self.output = Some(output);
        self
    }

    pub fn with_stdin(mut self, stdin: crate::StdinContract) -> Self {
        self.stdin = Some(stdin);
        self
    }

    pub fn with_progress_phase(mut self, phase: crate::ProgressPhaseSpec) -> Self {
        self.progress.push(phase);
        self
    }

    pub fn with_permission(mut self, permission: PermissionSpec) -> Self {
        self.permissions.push(permission);
        self
    }

    /// Declares that the handler deduplicates re-issued invocations, making
    /// the command safe to retry after an ambiguous failure. The natural
    /// deduplication key is the plan's `invocation_fingerprint`.
    pub fn idempotent(mut self) -> Self {
        self.idempotent = true;
        self
    }

    pub fn task_support(mut self, support: crate::TaskSupportSpec) -> Self {
        self.task_support = support;
        self
    }

    pub fn with_example(mut self, example: CommandExample) -> Self {
        self.examples.push(example);
        self
    }

    /// Declares that this command requires the named workspace resolved,
    /// without a path argument. Planning fails when the workspace does not
    /// resolve; the handler observes the root through the plan. Declaring
    /// the same workspace twice is a no-op.
    pub fn uses_workspace(mut self, name: impl Into<String>) -> Self {
        let name = name.into();
        if !self.workspaces.contains(&name) {
            self.workspaces.push(name);
        }
        self
    }

    /// Declares that the handler can consume the named workspace when the
    /// host supplies one. Absence remains valid.
    pub fn uses_optional_workspace(mut self, name: impl Into<String>) -> Self {
        let name = name.into();
        if !self.optional_workspaces.contains(&name) {
            self.optional_workspaces.push(name);
        }
        self
    }

    /// Declares that this command can consume optional host-supplied
    /// conversation identity through `CommandContext`.
    pub fn uses_conversation_identity(mut self) -> Self {
        self.uses_conversation_identity = true;
        self
    }

    /// Declares that this command requires the named capability. The
    /// capability's carrier argument must be declared as a required
    /// argument. Declaring the same requirement twice is a no-op.
    pub fn requires(mut self, capability: impl Into<String>) -> Self {
        let capability = capability.into();
        if !self.requires.contains(&capability) {
            self.requires.push(capability);
        }
        self
    }

    /// Declares that this command establishes the named capability.
    /// Declaring the same capability twice is a no-op.
    pub fn provides(mut self, capability: impl Into<String>) -> Self {
        let capability = capability.into();
        if !self.provides.contains(&capability) {
            self.provides.push(capability);
        }
        self
    }

    /// One sentence: when this command is the right choice. Positive
    /// polarity; mutually exclusive with `fallback`.
    pub fn use_when(mut self, text: impl Into<String>) -> Self {
        self.use_when = Some(text.into());
        self
    }

    /// Declares a routing edge to the command serving a neighboring case,
    /// with the condition that routes there.
    pub fn alternative(mut self, command: impl Into<String>, when: impl Into<String>) -> Self {
        self.alternatives.push(Alternative {
            command: command.into(),
            when: when.into(),
        });
        self
    }

    /// Marks this command as an escape hatch: the commands to exhaust
    /// first and the condition that justifies bypassing them. Mutually
    /// exclusive with `use_when` — the fallback's condition is its
    /// selection criterion. Preferences are copied from borrowed or owned
    /// string-like items into the catalog declaration.
    pub fn fallback(
        mut self,
        prefer: impl IntoIterator<Item = impl AsRef<str>>,
        when: impl Into<String>,
    ) -> Self {
        self.fallback = Some(Fallback {
            prefer: prefer
                .into_iter()
                .map(|preferred| preferred.as_ref().to_string())
                .collect(),
            when: when.into(),
        });
        self
    }

    pub fn arg(&self, name: &str) -> Option<&ArgSpec> {
        self.args.iter().find(|arg| arg.name == name)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum OutputFormat {
    Structured,
    Text,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum ResponseProfile {
    Text,
    Structured,
    CompactStructured,
    Debug,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct OutputSpec {
    #[serde(default = "default_output_format")]
    pub format: OutputFormat,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<ResponseProfile>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fields: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
    #[serde(default, rename = "maxBytes", skip_serializing_if = "Option::is_none")]
    pub max_bytes: Option<usize>,
}

impl Default for OutputSpec {
    fn default() -> Self {
        Self {
            format: OutputFormat::Structured,
            profile: None,
            limit: None,
            fields: None,
            cursor: None,
            max_bytes: Some(32 * 1024),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct StdinSpec {
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "camelCase")]
pub enum RunMode {
    #[default]
    Execute,
    Preview,
    DryRun,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalInput {
    pub token: String,
    #[serde(default)]
    pub confirm: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RunRequest {
    pub command: String,
    #[serde(default)]
    pub args: BTreeMap<String, Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stdin: Option<StdinSpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<OutputSpec>,
    #[serde(default)]
    pub mode: RunMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approval: Option<ApprovalInput>,
    #[serde(default)]
    pub dry_run: bool,
}

impl RunRequest {
    pub fn effective_mode(&self) -> RunMode {
        if self.dry_run {
            RunMode::DryRun
        } else {
            self.mode
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct BoundArg {
    pub name: String,
    pub value_type: ArgType,
    pub value: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<String>,
    /// The matched variant name(s) when `value_type` is `Named`. Per-element
    /// for repeated arguments. Participates in the invocation fingerprint.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub variants: Option<ArgVariants>,
    #[serde(default, skip_serializing_if = "crate::ArgSchemaMatch::is_empty")]
    pub schema_match: crate::ArgSchemaMatch,
}

/// Which union variant a bound `Named` argument matched.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum ArgVariants {
    Single(String),
    PerElement(Vec<String>),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum InvocationToken {
    Literal { value: String },
    Placeholder { name: String, value: Value },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "camelCase")]
pub enum InvocationOrigin {
    #[default]
    CommandTemplate,
    OperationId,
}

pub(crate) fn is_command_template(origin: &InvocationOrigin) -> bool {
    matches!(origin, InvocationOrigin::CommandTemplate)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ServingSurfaceIdentity {
    pub name: String,
    pub hash: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ServingSurfaceIdentityWire {
    name: String,
    hash: String,
}

impl ServingSurfaceIdentity {
    pub fn new(name: impl Into<String>, hash: impl Into<String>) -> crate::Result<Self> {
        let identity = Self {
            name: name.into(),
            hash: hash.into(),
        };
        identity.validate()?;
        Ok(identity)
    }

    fn validate(&self) -> crate::Result<()> {
        let valid_name = !self.name.is_empty()
            && self.name.len() <= 64
            && self.name.is_ascii()
            && self.name.split('-').all(|part| {
                !part.is_empty()
                    && part
                        .bytes()
                        .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
            });
        if !valid_name {
            return Err(crate::FrameworkError::Build(
                "serving surface name must be 1-64 lowercase kebab-case ASCII characters"
                    .to_string(),
            ));
        }
        if self.hash.len() != 64
            || !self
                .hash
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(crate::FrameworkError::Build(
                "serving surface hash must be a lowercase SHA-256 digest".to_string(),
            ));
        }
        Ok(())
    }
}

impl<'de> Deserialize<'de> for ServingSurfaceIdentity {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = ServingSurfaceIdentityWire::deserialize(deserializer)?;
        Self::new(wire.name, wire.hash).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct InvocationPlan {
    pub operation_id: String,
    pub command_path: Vec<String>,
    #[serde(default, skip_serializing_if = "is_command_template")]
    pub origin: InvocationOrigin,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_command: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub surface: Option<ServingSurfaceIdentity>,
    pub catalog_hash: String,
    pub invocation_fingerprint: String,
    pub effect: crate::EffectSpec,
    pub lane: crate::EffectLane,
    pub tokens: Vec<InvocationToken>,
    pub bound_args: BTreeMap<String, BoundArg>,
    pub permissions: Vec<PermissionSpec>,
    pub workspaces: Vec<WorkspaceDecl>,
    /// The resolved roots for the workspaces this plan's path arguments use.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub workspace_roots: Vec<PlanWorkspaceRoot>,
    /// Redacted resource-binding sources selected by the active native
    /// surface. Bare-registry and effect-lane plans leave this empty.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub resource_binding_facts: Vec<crate::PlanResourceBindingFact>,
    /// Whether the command declared itself idempotent; retry policy reads
    /// this from the plan instead of trusting a supervisor's judgment.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub idempotent: bool,
    pub output: OutputSpec,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PermissionDecision {
    Allow,
    RequireConfirmation,
    Deny { reason: String },
}

pub trait PermissionAuthorizer: Send + Sync {
    fn decide(&self, plan: &InvocationPlan) -> PermissionDecision;
}

#[derive(Debug, Clone, Default)]
pub struct DefaultPermissionAuthorizer;

impl PermissionAuthorizer for DefaultPermissionAuthorizer {
    fn decide(&self, plan: &InvocationPlan) -> PermissionDecision {
        decision_for_effect(&plan.effect)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "camelCase")]
pub enum ConfirmationPolicy {
    #[default]
    EffectDefault,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ReplayRecord {
    pub token: String,
    pub invocation_fingerprint: String,
    pub operation_id: String,
    pub command_path: Vec<String>,
    pub lane: crate::EffectLane,
    pub issued_at_unix_ms: i64,
    pub expires_at_unix_ms: i64,
    pub single_use: bool,
}

fn decision_for_effect(effect: &crate::EffectSpec) -> PermissionDecision {
    match effect {
        crate::EffectSpec::Pure | crate::EffectSpec::Read => PermissionDecision::Allow,
        crate::EffectSpec::Write
        | crate::EffectSpec::Delete
        | crate::EffectSpec::Exec
        | crate::EffectSpec::Network => PermissionDecision::RequireConfirmation,
        crate::EffectSpec::Custom(value) => PermissionDecision::Deny {
            reason: format!("custom effect `{value}` does not have a configured authorizer"),
        },
        crate::EffectSpec::Composite(effects) => {
            let mut requires_confirmation = false;
            for effect in effects {
                match decision_for_effect(effect) {
                    PermissionDecision::Allow => {}
                    PermissionDecision::RequireConfirmation => requires_confirmation = true,
                    PermissionDecision::Deny { reason } => {
                        return PermissionDecision::Deny { reason };
                    }
                }
            }
            if requires_confirmation {
                PermissionDecision::RequireConfirmation
            } else {
                PermissionDecision::Allow
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CommandOutput {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub structured: Option<Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub stderr: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
    /// References this call granted, minted by the framework from the
    /// handler's typed output (RFC 0012).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub grants: Vec<ResourceRef>,
    /// References this call enumerated, minted by the framework from the
    /// handler's typed output (RFC 0012).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub listings: Vec<ResourceRef>,
}

impl CommandOutput {
    pub fn structured(value: Value) -> Self {
        Self {
            text: None,
            structured: Some(value),
            stderr: Vec::new(),
            next_cursor: None,
            grants: Vec::new(),
            listings: Vec::new(),
        }
    }

    pub fn text(value: impl Into<String>) -> Self {
        Self {
            text: Some(value.into()),
            structured: None,
            stderr: Vec::new(),
            next_cursor: None,
            grants: Vec::new(),
            listings: Vec::new(),
        }
    }

    pub fn apply_output_spec(mut self, spec: &OutputSpec) -> Self {
        if let Some(value) = self.structured.take() {
            self.structured = Some(limit_structured(
                shape_structured(value, spec),
                spec.max_bytes,
            ));
        }

        if let Some(text) = self.text.take() {
            self.text = Some(limit_text(text, spec.max_bytes));
        }

        // Listings honor the row limit like any other enumeration; the
        // full set stays recoverable by re-asking the enumerator. Grants
        // are never truncated — dropping an acquisition record would
        // orphan a live lease.
        if let Some(limit) = spec.limit {
            self.listings.truncate(limit);
        }

        self
    }

    pub(crate) fn compact_text_from_structured(
        mut self,
        max_bytes: Option<usize>,
    ) -> serde_json::Result<Self> {
        self.text = self
            .structured
            .as_ref()
            .map(serde_json::to_string)
            .transpose()?
            .map(|text| limit_text(text, max_bytes));
        Ok(self)
    }
}

fn shape_structured(value: Value, spec: &OutputSpec) -> Value {
    let limited = match (value, spec.limit) {
        (Value::Array(items), Some(limit)) => Value::Array(items.into_iter().take(limit).collect()),
        (value, _) => value,
    };

    match spec.fields.as_ref() {
        Some(fields) => select_fields(limited, fields),
        None => limited,
    }
}

fn select_fields(value: Value, fields: &[String]) -> Value {
    match value {
        Value::Array(items) => Value::Array(
            items
                .into_iter()
                .map(|item| select_fields(item, fields))
                .collect(),
        ),
        Value::Object(map) => {
            let mut selected = serde_json::Map::new();
            for field in fields {
                if let Some(value) = map.get(field) {
                    selected.insert(field.clone(), value.clone());
                }
            }
            Value::Object(selected)
        }
        other => other,
    }
}

fn limit_structured(value: Value, max_bytes: Option<usize>) -> Value {
    let Some(max_bytes) = max_bytes else {
        return value;
    };
    let Ok(bytes) = serde_json::to_vec(&value) else {
        return value;
    };
    if bytes.len() <= max_bytes {
        return value;
    }

    let preview = String::from_utf8_lossy(&bytes).to_string();
    json!({
        "truncated": true,
        "maxBytes": max_bytes,
        "actualBytes": bytes.len(),
        "preview": limit_text(preview, Some(max_bytes)),
    })
}

fn limit_text(text: String, max_bytes: Option<usize>) -> String {
    let Some(max_bytes) = max_bytes else {
        return text;
    };
    if text.len() <= max_bytes {
        return text;
    }

    let marker = "...[truncated]";
    let target = max_bytes.saturating_sub(marker.len());
    let mut end = if target == 0 { max_bytes } else { target };
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    if target == 0 {
        text[..end].to_string()
    } else {
        format!("{}{}", &text[..end], marker)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RunResponse {
    pub plan: InvocationPlan,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<CommandOutput>,
    pub dry_run: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CommandContext {
    pub plan: InvocationPlan,
    pub stdin: Option<StdinSpec>,
    /// The resources the framework resolved for this invocation (RFC
    /// 0012). Extractor parameters read from here; the field never
    /// serializes because resolved values are live server-side state.
    #[serde(skip)]
    #[schemars(skip)]
    pub resources: crate::ResolvedResources,
    /// Private host facts for this invocation. Values are structurally
    /// excluded from both serialization and schema generation.
    #[serde(skip)]
    #[schemars(skip)]
    invocation_context: crate::InvocationContext,
    /// Whether this invocation's selected command activated RFC 0017's
    /// checked typed-extraction boundary. This is dispatch-only state and is
    /// structurally excluded from plans, responses, and schemas.
    #[serde(skip)]
    #[schemars(skip)]
    checked_argument_contract: bool,
}

impl CommandContext {
    pub fn new(
        plan: InvocationPlan,
        stdin: Option<StdinSpec>,
        resources: crate::ResolvedResources,
    ) -> Self {
        Self {
            plan,
            stdin,
            resources,
            invocation_context: crate::InvocationContext::default(),
            checked_argument_contract: false,
        }
    }

    pub(crate) fn with_invocation_context(
        plan: InvocationPlan,
        stdin: Option<StdinSpec>,
        resources: crate::ResolvedResources,
        invocation_context: crate::InvocationContext,
    ) -> Self {
        Self {
            plan,
            stdin,
            resources,
            invocation_context,
            checked_argument_contract: false,
        }
    }

    pub(crate) fn with_checked_argument_contract(mut self) -> Self {
        self.checked_argument_contract = true;
        self
    }

    pub(crate) fn checked_argument_contract(&self) -> bool {
        self.checked_argument_contract
    }

    pub fn conversation_identity(&self) -> Option<&crate::ConversationIdentity> {
        self.invocation_context.conversation_identity()
    }

    /// The resolved root for a workspace this command declared or one of
    /// its path arguments referenced. Planning guarantees presence for
    /// declared workspaces; path-argument workspaces are present when a
    /// bound argument referenced them.
    pub fn workspace_root(&self, id: &str) -> Option<&PlanWorkspaceRoot> {
        self.plan.workspace_roots.iter().find(|root| root.id == id)
    }
}

#[derive(Debug, Clone)]
pub struct PermissionPolicy {
    allowed: BTreeSet<PermissionEffect>,
}

impl PermissionPolicy {
    pub fn allow_all() -> Self {
        Self {
            allowed: [
                PermissionEffect::Read,
                PermissionEffect::Write,
                PermissionEffect::Delete,
                PermissionEffect::Exec,
                PermissionEffect::Network,
            ]
            .into_iter()
            .collect(),
        }
    }

    pub fn read_only() -> Self {
        Self {
            allowed: [PermissionEffect::Read].into_iter().collect(),
        }
    }

    pub fn allows(&self, permission: &PermissionSpec) -> bool {
        self.allowed.contains(&permission.effect)
            || matches!(
                &permission.effect,
                PermissionEffect::Custom(effect) if self.allowed.contains(&PermissionEffect::Custom(effect.clone()))
            )
    }

    pub fn check(&self, permissions: &[PermissionSpec]) -> Result<()> {
        for permission in permissions {
            if !self.allows(permission) {
                return Err(FrameworkError::PermissionDenied {
                    effect: permission.effect.as_label(),
                    scope: permission.scope.clone(),
                });
            }
        }
        Ok(())
    }
}

impl Default for PermissionPolicy {
    fn default() -> Self {
        Self::allow_all()
    }
}

pub fn value_matches_type(name: &str, value: &Value, value_type: &ArgType) -> Result<()> {
    let valid = match value_type {
        ArgType::String | ArgType::Path => value.is_string(),
        ArgType::Json => true,
        ArgType::Bool => value.is_boolean(),
        ArgType::Number => value.is_number(),
        ArgType::Integer => value
            .as_number()
            .is_some_and(crate::JsonInteger::number_is_integer),
        // Named types are matched against their declared variants by the
        // planner, which has the type table this function does not.
        ArgType::Named(_) => true,
        // Resource references are strings (bare id or URI); liveness is
        // the resolver's judgment at extraction time.
        ArgType::ResourceRef(_) => value.is_string(),
    };
    if valid {
        Ok(())
    } else {
        Err(FrameworkError::InvalidArgumentType(
            name.to_string(),
            value_type.expected_name(),
        ))
    }
}

pub fn structured_error(error: &FrameworkError) -> Value {
    json!({
        "error": error.to_string()
    })
}
