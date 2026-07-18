//! Ambient resource binding (RFC 0016).
//!
//! The public declarations describe how a compiled native surface obtains a
//! resource reference. Concrete binders remain private runtime sidecars. A
//! binding is selected while planning and realized only after authorization.

use std::{error::Error, fmt, future::Future, marker::PhantomData, pin::Pin, sync::Arc};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{
    AllApplicationErrorCodes, ApplicationError, ApplicationErrorFootprint, ConversationIdentity,
    FrameworkError, PlanWorkspaceRoot, ProducedApplicationError, Resource, ResourceRefusal,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ResourceBindingDecl {
    pub resource: String,
    pub mode: ResourceBindingMode,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum ResourceBindingMode {
    Argument,
    Ambient {
        context: AmbientContextSource,
        explicit: ExplicitCarrierPolicy,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        missing_error: Option<String>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum AmbientContextSource {
    ConversationIdentity,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum ExplicitCarrierPolicy {
    Omitted,
    OptionalOverride,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PlanResourceBindingFact {
    pub resource: String,
    pub source: PlanResourceBindingSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum PlanResourceBindingSource {
    Argument,
    Ambient,
    Absent,
}

/// Fluent fresh-authoring value pairing one serializable binding declaration
/// with its private runtime binder.
pub struct AmbientResourceBinding<B> {
    binder: B,
    explicit: Option<ExplicitCarrierPolicy>,
    missing_error: Option<String>,
    errors: Vec<&'static str>,
}

impl<B> AmbientResourceBinding<B> {
    pub fn from_conversation_identity(binder: B) -> Self {
        Self {
            binder,
            explicit: None,
            missing_error: None,
            errors: Vec::new(),
        }
    }

    pub fn with_optional_explicit_carrier(mut self) -> Self {
        self.set_explicit(ExplicitCarrierPolicy::OptionalOverride);
        self
    }

    pub fn omit_explicit_carrier(mut self) -> Self {
        self.set_explicit(ExplicitCarrierPolicy::Omitted);
        self
    }

    pub fn missing_as(mut self, application_code: impl Into<String>) -> Self {
        if self.missing_error.is_some() {
            self.errors
                .push("ambient resource binding assigns `missing_as` more than once");
        } else {
            self.missing_error = Some(application_code.into());
        }
        self
    }

    fn set_explicit(&mut self, policy: ExplicitCarrierPolicy) {
        if self.explicit.is_some() {
            self.errors
                .push("ambient resource binding assigns an explicit-carrier policy more than once");
        } else {
            self.explicit = Some(policy);
        }
    }

    pub(crate) fn into_runtime<T>(
        self,
    ) -> crate::Result<(
        ResourceBindingDecl,
        Arc<dyn ErasedAmbientBinder>,
        Vec<String>,
    )>
    where
        T: Resource,
        B: BindAmbientResource<T>,
    {
        if let Some(error) = self.errors.into_iter().next() {
            return Err(FrameworkError::Build(error.to_string()));
        }
        let explicit = self.explicit.ok_or_else(|| {
            FrameworkError::Build(
                "ambient resource binding must explicitly choose carrier omission or optional override"
                    .to_string(),
            )
        })?;
        let codes = B::ErrorFootprint::codes()
            .into_iter()
            .map(ToOwned::to_owned)
            .collect();
        Ok((
            ResourceBindingDecl {
                resource: T::NAME.to_string(),
                mode: ResourceBindingMode::Ambient {
                    context: AmbientContextSource::ConversationIdentity,
                    explicit,
                    missing_error: self.missing_error,
                },
            },
            Arc::new(AmbientBinderAdapter::<T, B> {
                binder: self.binder,
                marker: PhantomData,
            }),
            codes,
        ))
    }
}

pub trait BindAmbientResource<T: Resource>: Send + Sync + 'static {
    type Error: ApplicationError;
    type ErrorFootprint: ApplicationErrorFootprint<Self::Error>;

    fn bind(
        &self,
        context: AmbientBindingContext<'_>,
    ) -> impl Future<
        Output = std::result::Result<
            PrivateResourceReference,
            AmbientBindingFailure<Self::Error, Self::ErrorFootprint>,
        >,
    > + Send;
}

pub struct AmbientBindingContext<'a> {
    pub operation_id: &'a str,
    pub conversation_identity: &'a ConversationIdentity,
    pub workspaces: &'a [PlanWorkspaceRoot],
}

pub enum AmbientBindingFailure<E, F = AllApplicationErrorCodes<E>>
where
    E: ApplicationError,
    F: ApplicationErrorFootprint<E>,
{
    Application(ProducedApplicationError<E, F>),
    Infrastructure(AmbientBindingInfrastructureError),
}

impl<E, F> From<E> for AmbientBindingFailure<E, F>
where
    E: ApplicationError,
    F: ApplicationErrorFootprint<E>,
{
    fn from(error: E) -> Self {
        Self::Application(error.into())
    }
}

impl<E, F> From<PrivateResourceReferenceError> for AmbientBindingFailure<E, F>
where
    E: ApplicationError,
    F: ApplicationErrorFootprint<E>,
{
    fn from(error: PrivateResourceReferenceError) -> Self {
        Self::Infrastructure(AmbientBindingInfrastructureError::new(error))
    }
}

impl<E, F> From<AmbientBindingInfrastructureError> for AmbientBindingFailure<E, F>
where
    E: ApplicationError,
    F: ApplicationErrorFootprint<E>,
{
    fn from(error: AmbientBindingInfrastructureError) -> Self {
        Self::Infrastructure(error)
    }
}

pub struct AmbientBindingInfrastructureError {
    _source: Box<dyn Error + Send + Sync + 'static>,
}

impl AmbientBindingInfrastructureError {
    pub fn new(source: impl Error + Send + Sync + 'static) -> Self {
        Self {
            _source: Box::new(source),
        }
    }
}

impl fmt::Debug for AmbientBindingInfrastructureError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("AmbientBindingInfrastructureError(<redacted>)")
    }
}

impl fmt::Display for AmbientBindingInfrastructureError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ambient resource binding failed")
    }
}

impl Error for AmbientBindingInfrastructureError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        None
    }
}

pub struct PrivateResourceReference {
    id: String,
}

impl PrivateResourceReference {
    pub fn from_id(
        id: impl Into<String>,
    ) -> std::result::Result<Self, PrivateResourceReferenceError> {
        let id = id.into();
        if id.is_empty() {
            return Err(PrivateResourceReferenceError::Empty);
        }
        if !id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~'))
        {
            return Err(PrivateResourceReferenceError::InvalidCharacter);
        }
        Ok(Self { id })
    }

    pub(crate) fn as_id(&self) -> &str {
        &self.id
    }
}

impl fmt::Debug for PrivateResourceReference {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PrivateResourceReference(<redacted>)")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrivateResourceReferenceError {
    Empty,
    InvalidCharacter,
}

impl fmt::Display for PrivateResourceReferenceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Empty => "private resource reference is empty",
            Self::InvalidCharacter => "private resource reference contains an invalid character",
        })
    }
}

impl Error for PrivateResourceReferenceError {}

pub enum ResourceResolutionFailure<E, F = AllApplicationErrorCodes<E>>
where
    E: ApplicationError,
    F: ApplicationErrorFootprint<E>,
{
    Refused(ResourceRefusal),
    Application(ProducedApplicationError<E, F>),
}

impl<E, F> From<E> for ResourceResolutionFailure<E, F>
where
    E: ApplicationError,
    F: ApplicationErrorFootprint<E>,
{
    fn from(error: E) -> Self {
        Self::Application(error.into())
    }
}

pub trait ResolveResourceWithErrors<T: Resource>: Send + Sync + 'static {
    type Error: ApplicationError;
    type ErrorFootprint: ApplicationErrorFootprint<Self::Error>;

    fn resolve(
        &self,
        reference: &str,
        plan: &crate::InvocationPlan,
    ) -> impl Future<
        Output = std::result::Result<
            T,
            ResourceResolutionFailure<Self::Error, Self::ErrorFootprint>,
        >,
    > + Send;
}

pub(crate) enum ErasedAmbientBindingFailure {
    Application(crate::results::RawApplicationError),
    Infrastructure,
}

type AmbientBindFuture<'a> = Pin<
    Box<
        dyn Future<
                Output = std::result::Result<PrivateResourceReference, ErasedAmbientBindingFailure>,
            > + Send
            + 'a,
    >,
>;

pub(crate) trait ErasedAmbientBinder: Send + Sync {
    fn bind_erased<'a>(&'a self, context: AmbientBindingContext<'a>) -> AmbientBindFuture<'a>;
}

pub(crate) enum PreparedAbsentBinding {
    Optional,
    RequiredFramework,
    RequiredApplication(crate::ApplicationErrorBody),
}

pub(crate) enum PlannedResourceBinding {
    Argument {
        resource: String,
    },
    Ambient {
        resource: String,
        binder: Arc<dyn ErasedAmbientBinder>,
    },
    Absent {
        resource: String,
        behavior: PreparedAbsentBinding,
    },
}

pub(crate) struct PreparedInvocation {
    pub(crate) plan: crate::InvocationPlan,
    pub(crate) invocation_context: crate::InvocationContext,
    pub(crate) resource_bindings: Vec<PlannedResourceBinding>,
}

impl PreparedInvocation {
    pub(crate) fn argument_bound(
        plan: crate::InvocationPlan,
        invocation_context: crate::InvocationContext,
    ) -> Self {
        Self {
            plan,
            invocation_context,
            resource_bindings: Vec::new(),
        }
    }

    pub(crate) fn plan(&self) -> &crate::InvocationPlan {
        &self.plan
    }

    pub(crate) fn into_plan(self) -> crate::InvocationPlan {
        self.plan
    }
}

impl fmt::Debug for PreparedInvocation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PreparedInvocation")
            .field("plan", &self.plan)
            .field("invocation_context", &"<redacted>")
            .field("resource_bindings", &self.resource_bindings.len())
            .finish()
    }
}

pub(crate) struct AmbientBinderAdapter<T, B> {
    binder: B,
    marker: PhantomData<fn() -> T>,
}

impl<T, B> AmbientBinderAdapter<T, B> {
    pub(crate) fn new(binder: B) -> Self {
        Self {
            binder,
            marker: PhantomData,
        }
    }
}

impl<T, B> ErasedAmbientBinder for AmbientBinderAdapter<T, B>
where
    T: Resource,
    B: BindAmbientResource<T>,
{
    fn bind_erased<'a>(&'a self, context: AmbientBindingContext<'a>) -> AmbientBindFuture<'a> {
        Box::pin(async move {
            match self.binder.bind(context).await {
                Ok(reference) => Ok(reference),
                Err(AmbientBindingFailure::Application(error)) => {
                    Err(ErasedAmbientBindingFailure::Application(error.into_raw()))
                }
                Err(AmbientBindingFailure::Infrastructure(_)) => {
                    Err(ErasedAmbientBindingFailure::Infrastructure)
                }
            }
        })
    }
}
