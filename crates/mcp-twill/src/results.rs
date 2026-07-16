use std::{
    any::TypeId,
    borrow::Cow,
    collections::{BTreeMap, BTreeSet},
    error::Error,
    fmt,
    future::Future,
    marker::PhantomData,
    sync::{Arc, Mutex, OnceLock},
};

use async_trait::async_trait;
use schemars::{JsonSchema, generate::SchemaSettings};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Map, Value, json};

use crate::{
    CommandContext, CommandOutput, FrameworkError, FromCommandArgs, Grant, Granted, Listed,
    Listing, Resource, ResourceRef,
    resource::{
        ContextAndArgs, ContextOnly, ResourceParams, ResourceUse, WithResources,
        WithResourcesAndArgs,
    },
};

const JSON_SCHEMA_DIALECT: &str = "https://json-schema.org/draft/2020-12/schema";
const MAX_PUBLIC_SCALARS: usize = 512;
type NumericShape = (Value, Option<Value>, Option<Value>);
type RustStorageShapes = BTreeMap<String, NumericShape>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ApplicationResultContract {
    pub success_schema: Value,
    pub errors: Vec<ApplicationErrorSpec>,
}

impl ApplicationResultContract {
    pub fn new(success_schema: impl Into<Value>) -> Self {
        Self {
            success_schema: success_schema.into(),
            errors: Vec::new(),
        }
    }

    pub fn for_type<T>() -> Self
    where
        T: Serialize + JsonSchema + Send + 'static,
    {
        let generator = SchemaSettings::draft2020_12().into_generator();
        let schema = generator.into_root_schema_for::<T>();
        let mut value = serde_json::to_value(schema).expect("Schemars schema serializes as JSON");
        normalize_typed_schema(&mut value);
        Self::new(value)
    }

    pub fn with_errors<E, S>(mut self) -> crate::Result<Self>
    where
        E: ApplicationError,
        S: ApplicationErrorSet<E>,
    {
        let declarations = E::declarations();
        let uses = S::uses();
        if uses.iter().any(|usage| usage.capability.is_some()) {
            return Err(FrameworkError::Build(
                "standalone application result contracts cannot resolve capability-bound errors"
                    .to_string(),
            ));
        }
        self.errors
            .extend(compose_error_specs(&declarations, &uses, |_| None)?);
        canonicalize_error_specs(&mut self.errors)?;
        Ok(self)
    }

    pub fn with_error_spec(mut self, error: ApplicationErrorSpec) -> Self {
        self.errors.push(error);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ApplicationErrorDecl {
    pub code: String,
    pub summary: String,
    pub message: ApplicationMessageDecl,
    pub details_schema: Value,
}

impl ApplicationErrorDecl {
    pub fn new(code: impl Into<String>, summary: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            summary: summary.into(),
            message: ApplicationMessageDecl::DeclarationSummary,
            details_schema: closed_empty_object_schema(),
        }
    }

    pub fn details_schema(mut self, schema: impl Into<Value>) -> Self {
        self.details_schema = schema.into();
        self
    }

    pub fn runtime_message(mut self, max_scalar_values: u16) -> Self {
        self.message = ApplicationMessageDecl::RuntimeBounded { max_scalar_values };
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ApplicationErrorUse {
    pub code: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capability: Option<String>,
    pub recoveries: Vec<ApplicationRecoveryDecl>,
    pub recovery_cardinality: RecoveryCardinality,
}

impl ApplicationErrorUse {
    pub fn new(code: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            capability: None,
            recoveries: Vec::new(),
            recovery_cardinality: RecoveryCardinality::Any,
        }
    }

    pub fn for_capability(mut self, capability: impl Into<String>) -> Self {
        self.capability = Some(capability.into());
        self
    }

    pub fn recover_with(mut self, operation_id: impl Into<String>) -> Self {
        self.recoveries.push(ApplicationRecoveryDecl::Operation {
            operation_id: operation_id.into(),
        });
        self
    }

    pub fn recover_by(
        mut self,
        action_code: impl Into<String>,
        summary: impl Into<String>,
    ) -> Self {
        self.recoveries
            .push(ApplicationRecoveryDecl::Action(ApplicationActionDecl {
                code: action_code.into(),
                summary: summary.into(),
            }));
        self
    }

    pub fn at_most_one_recovery(mut self) -> Self {
        self.recovery_cardinality = RecoveryCardinality::AtMostOne;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ApplicationErrorSpec {
    pub code: String,
    pub summary: String,
    pub message: ApplicationMessageDecl,
    pub details_schema: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capability: Option<String>,
    pub recoveries: Vec<ApplicationRecoveryDecl>,
    pub recovery_cardinality: RecoveryCardinality,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum ApplicationMessageDecl {
    DeclarationSummary,
    RuntimeBounded { max_scalar_values: u16 },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ApplicationActionDecl {
    pub code: String,
    pub summary: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum ApplicationRecoveryDecl {
    Operation { operation_id: String },
    Action(ApplicationActionDecl),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApplicationRecoveryKey {
    Operation(String),
    Action(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApplicationRecoverySelection {
    Declared,
    None,
    Only(Vec<ApplicationRecoveryKey>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum RecoveryCardinality {
    Any,
    AtMostOne,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ApplicationErrorBody {
    pub code: String,
    pub message: String,
    pub details: Value,
    pub recoveries: Vec<ApplicationRecovery>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum ApplicationRecovery {
    Operation { operation_id: String },
    Action { code: String, summary: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum CommandExecutionOutcome {
    Success(crate::RunResponse),
    ApplicationError {
        plan: crate::InvocationPlan,
        error: ApplicationErrorBody,
    },
}

pub type ApplicationResult<T, E = NoApplicationError, S = AllApplicationErrors<E>> =
    ApplicationOutputResult<ApplicationSuccess<T>, E, S>;

pub type ApplicationOutputResult<O, E = NoApplicationError, S = AllApplicationErrors<E>> =
    std::result::Result<O, CommandFailure<E, S>>;

#[derive(Debug)]
pub enum NoApplicationError {}

impl fmt::Display for NoApplicationError {
    fn fmt(&self, _: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {}
    }
}

impl Error for NoApplicationError {}

impl ApplicationError for NoApplicationError {
    fn declarations() -> Vec<ApplicationErrorDecl> {
        Vec::new()
    }

    fn code(&self) -> &'static str {
        match *self {}
    }

    fn details(&self) -> Value {
        match *self {}
    }
}

pub struct DeclaredApplicationError<E, S> {
    error: E,
    set: PhantomData<fn() -> S>,
}

pub enum CommandFailure<E, S = AllApplicationErrors<E>> {
    Application(DeclaredApplicationError<E, S>),
    Framework(FrameworkError),
}

impl<E, S> From<E> for CommandFailure<E, S>
where
    E: ApplicationError,
    S: ApplicationErrorSet<E>,
{
    fn from(error: E) -> Self {
        Self::Application(DeclaredApplicationError {
            error,
            set: PhantomData,
        })
    }
}

impl<E, S> From<FrameworkError> for CommandFailure<E, S>
where
    E: ApplicationError,
    S: ApplicationErrorSet<E>,
{
    fn from(error: FrameworkError) -> Self {
        Self::Framework(error)
    }
}

pub type DynamicApplicationResult<O = ApplicationSuccess<Value>> =
    std::result::Result<O, DynamicCommandFailure>;

pub enum DynamicCommandFailure {
    Application(DynamicApplicationError),
    Framework(FrameworkError),
}

impl From<DynamicApplicationError> for DynamicCommandFailure {
    fn from(error: DynamicApplicationError) -> Self {
        Self::Application(error)
    }
}

impl From<FrameworkError> for DynamicCommandFailure {
    fn from(error: FrameworkError) -> Self {
        Self::Framework(error)
    }
}

pub struct DynamicApplicationError {
    code: String,
    message: Option<String>,
    details: Value,
    recovery: ApplicationRecoverySelection,
}

impl DynamicApplicationError {
    pub fn new(code: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: None,
            details: json!({}),
            recovery: ApplicationRecoverySelection::Declared,
        }
    }

    pub fn message(mut self, message: impl Into<String>) -> Self {
        self.message = Some(message.into());
        self
    }

    pub fn details(mut self, details: Value) -> Self {
        self.details = details;
        self
    }

    pub fn recovery(mut self, selection: ApplicationRecoverySelection) -> Self {
        self.recovery = selection;
        self
    }
}

pub trait ApplicationError: Error + Send + Sync + 'static {
    fn declarations() -> Vec<ApplicationErrorDecl>;
    fn code(&self) -> &'static str;
    fn details(&self) -> Value;
    fn runtime_message(&self) -> Option<Cow<'_, str>> {
        None
    }
    fn recovery(&self) -> ApplicationRecoverySelection {
        ApplicationRecoverySelection::Declared
    }
}

pub trait ApplicationErrorSet<E: ApplicationError>: Send + Sync + 'static {
    fn uses() -> Vec<ApplicationErrorUse>;
}

pub struct AllApplicationErrors<E>(PhantomData<fn() -> E>);

impl<E: ApplicationError> ApplicationErrorSet<E> for AllApplicationErrors<E> {
    fn uses() -> Vec<ApplicationErrorUse> {
        E::declarations()
            .into_iter()
            .map(|decl| ApplicationErrorUse::new(decl.code))
            .collect()
    }
}

pub trait ApplicationErrorFootprint<E: ApplicationError>: Send + Sync + 'static {
    fn codes() -> Vec<&'static str>;
}

pub struct AllApplicationErrorCodes<E>(PhantomData<fn() -> E>);

impl<E: ApplicationError> ApplicationErrorFootprint<E> for AllApplicationErrorCodes<E> {
    fn codes() -> Vec<&'static str> {
        static CODES: OnceLock<Mutex<BTreeMap<TypeId, &'static [&'static str]>>> = OnceLock::new();
        let mut codes = CODES
            .get_or_init(|| Mutex::new(BTreeMap::new()))
            .lock()
            .expect("application error code cache poisoned");
        let cached = codes.entry(TypeId::of::<E>()).or_insert_with(|| {
            let values = E::declarations()
                .into_iter()
                .map(|decl| Box::leak(decl.code.into_boxed_str()) as &'static str)
                .collect::<Vec<_>>()
                .into_boxed_slice();
            Box::leak(values)
        });
        cached.to_vec()
    }
}

pub struct ProducedApplicationError<E, F> {
    _error: E,
    footprint: PhantomData<fn() -> F>,
}

impl<E, F> From<E> for ProducedApplicationError<E, F>
where
    E: ApplicationError,
    F: ApplicationErrorFootprint<E>,
{
    fn from(error: E) -> Self {
        Self {
            _error: error,
            footprint: PhantomData,
        }
    }
}

pub struct ApplicationSuccess<T> {
    value: T,
    resources: ApplicationResourceComponents,
}

#[derive(Default)]
struct ApplicationResourceComponents {
    grants: Vec<ResourceRef>,
    listings: Vec<ResourceRef>,
}

impl<T> ApplicationSuccess<T> {
    pub fn value(value: T) -> Self {
        Self {
            value,
            resources: ApplicationResourceComponents::default(),
        }
    }
}

impl<T> From<T> for ApplicationSuccess<T> {
    fn from(value: T) -> Self {
        Self::value(value)
    }
}

mod sealed {
    pub trait ApplicationOutput {}
    pub trait ResultDialect<M> {}
    pub trait DynamicDialect<M> {}
}

pub trait ApplicationOutput: sealed::ApplicationOutput + Send + 'static {
    type Value: Serialize + JsonSchema + Send + 'static;
    fn granted() -> Vec<&'static str>;
    fn enumerated() -> Vec<&'static str>;
    #[doc(hidden)]
    fn into_success(self) -> ApplicationSuccess<Self::Value>;

    fn grant<R: Resource>(self, grant: Grant<R>) -> Granted<R, Self>
    where
        Self: Sized,
    {
        Granted::new(self, grant)
    }

    fn listing<R: Resource>(self, listing: Listing<R>) -> Listed<R, Self>
    where
        Self: Sized,
    {
        Listed::new(self, listing)
    }
}

impl<T> sealed::ApplicationOutput for ApplicationSuccess<T> {}

impl<T> ApplicationOutput for ApplicationSuccess<T>
where
    T: Serialize + JsonSchema + Send + 'static,
{
    type Value = T;

    fn granted() -> Vec<&'static str> {
        Vec::new()
    }
    fn enumerated() -> Vec<&'static str> {
        Vec::new()
    }
    fn into_success(self) -> ApplicationSuccess<T> {
        self
    }
}

impl<R, O> sealed::ApplicationOutput for Granted<R, O>
where
    R: Resource,
    O: ApplicationOutput,
{
}

impl<R, O> ApplicationOutput for Granted<R, O>
where
    R: Resource,
    O: ApplicationOutput,
{
    type Value = O::Value;

    fn granted() -> Vec<&'static str> {
        let mut values = O::granted();
        values.push(R::NAME);
        values
    }
    fn enumerated() -> Vec<&'static str> {
        O::enumerated()
    }
    fn into_success(self) -> ApplicationSuccess<Self::Value> {
        let (output, grant) = self.into_parts();
        let mut success = output.into_success();
        success.resources.grants.push(ResourceRef {
            resource: R::NAME.to_string(),
            id: grant.into_id(),
            uri: String::new(),
        });
        success
    }
}

impl<R, O> sealed::ApplicationOutput for Listed<R, O>
where
    R: Resource,
    O: ApplicationOutput,
{
}

impl<R, O> ApplicationOutput for Listed<R, O>
where
    R: Resource,
    O: ApplicationOutput,
{
    type Value = O::Value;

    fn granted() -> Vec<&'static str> {
        O::granted()
    }
    fn enumerated() -> Vec<&'static str> {
        let mut values = O::enumerated();
        values.push(R::NAME);
        values
    }
    fn into_success(self) -> ApplicationSuccess<Self::Value> {
        let (output, listing) = self.into_parts();
        let mut success = output.into_success();
        success
            .resources
            .listings
            .extend(listing.into_ids().into_iter().map(|id| ResourceRef {
                resource: R::NAME.to_string(),
                id,
                uri: String::new(),
            }));
        success
    }
}

#[macro_export]
macro_rules! application_error_set {
    ($vis:vis struct $name:ident for $error:ty { $($use:expr),* $(,)? }) => {
        $vis struct $name;
        impl $crate::ApplicationErrorSet<$error> for $name {
            fn uses() -> Vec<$crate::ApplicationErrorUse> {
                vec![$($use),*]
            }
        }
    };
}

#[macro_export]
macro_rules! application_error_footprint {
    ($vis:vis struct $name:ident for $error:ty { $($code:expr),* $(,)? }) => {
        $vis struct $name;
        impl $crate::ApplicationErrorFootprint<$error> for $name {
            fn codes() -> Vec<&'static str> {
                vec![$($code),*]
            }
        }
    };
}

pub(crate) enum HandlerOutcome {
    Success(CommandOutput),
    ApplicationSuccess {
        value: Value,
        grants: Vec<ResourceRef>,
        listings: Vec<ResourceRef>,
    },
    ApplicationError(RawApplicationError),
}

pub(crate) struct RawApplicationError {
    pub code: String,
    pub message: Option<String>,
    pub details: Value,
    pub recovery: ApplicationRecoverySelection,
}

#[async_trait]
pub(crate) trait ErasedCommandHandler: Send + Sync + 'static {
    async fn call(&self, context: CommandContext) -> crate::Result<HandlerOutcome>;
}

pub(crate) struct LegacyHandlerAdapter<H>(pub H);

#[async_trait]
impl<H: crate::CommandHandler> ErasedCommandHandler for LegacyHandlerAdapter<H> {
    async fn call(&self, context: CommandContext) -> crate::Result<HandlerOutcome> {
        self.0.call(context).await.map(HandlerOutcome::Success)
    }
}

#[derive(Clone)]
pub(crate) struct PendingApplicationContract {
    pub success_schema: Value,
    pub declarations: Vec<ApplicationErrorDecl>,
    pub uses: Vec<ApplicationErrorUse>,
}

#[doc(hidden)]
pub struct ResultHandlerRegistration {
    pub(crate) handler: Arc<dyn ErasedCommandHandler>,
    pub(crate) pending: PendingApplicationContract,
    pub(crate) argument_schema: Option<Value>,
    pub(crate) resource_uses: Vec<ResourceUse>,
    pub(crate) granted: Vec<&'static str>,
    pub(crate) enumerated: Vec<&'static str>,
}

#[doc(hidden)]
pub struct DynamicHandlerRegistration {
    pub(crate) handler: Arc<dyn ErasedCommandHandler>,
    pub(crate) resource_uses: Vec<ResourceUse>,
    pub(crate) granted: Vec<&'static str>,
    pub(crate) enumerated: Vec<&'static str>,
}

pub trait ApplicationResultDialect<M>: sealed::ResultDialect<M> + Send + Sync + 'static {
    #[doc(hidden)]
    fn into_result_registration(self) -> ResultHandlerRegistration;
}

pub trait DynamicApplicationDialect<M>: sealed::DynamicDialect<M> + Send + Sync + 'static {
    #[doc(hidden)]
    fn into_dynamic_registration(self) -> DynamicHandlerRegistration;
}

struct TypedResultHandler<M, H> {
    handler: H,
    _marker: PhantomData<fn() -> M>,
}

struct DynamicResultHandler<M, H> {
    handler: H,
    _marker: PhantomData<fn() -> M>,
}

fn typed_registration<O, E, S>(
    handler: Arc<dyn ErasedCommandHandler>,
    resource_uses: Vec<ResourceUse>,
    argument_schema: Option<Value>,
) -> ResultHandlerRegistration
where
    O: ApplicationOutput,
    E: ApplicationError,
    S: ApplicationErrorSet<E>,
{
    ResultHandlerRegistration {
        handler,
        pending: PendingApplicationContract {
            success_schema: ApplicationResultContract::for_type::<O::Value>().success_schema,
            declarations: E::declarations(),
            uses: S::uses(),
        },
        argument_schema,
        resource_uses,
        granted: O::granted(),
        enumerated: O::enumerated(),
    }
}

fn dynamic_registration<O: ApplicationOutput>(
    handler: Arc<dyn ErasedCommandHandler>,
    resource_uses: Vec<ResourceUse>,
) -> DynamicHandlerRegistration {
    DynamicHandlerRegistration {
        handler,
        resource_uses,
        granted: O::granted(),
        enumerated: O::enumerated(),
    }
}

fn typed_failure<E: ApplicationError, S>(
    failure: CommandFailure<E, S>,
) -> crate::Result<HandlerOutcome> {
    match failure {
        CommandFailure::Framework(FrameworkError::CapabilityDenied { .. }) => {
            Err(FrameworkError::Handler(
                "result-aware handler returned legacy capability denial".to_string(),
            ))
        }
        CommandFailure::Framework(FrameworkError::ArgumentContractViolation { .. }) => {
            Err(FrameworkError::Handler(
                "handler returned invalid argument contract violation".to_string(),
            ))
        }
        CommandFailure::Framework(error) => Err(error),
        CommandFailure::Application(error) => {
            Ok(HandlerOutcome::ApplicationError(RawApplicationError {
                code: error.error.code().to_string(),
                message: error.error.runtime_message().map(Cow::into_owned),
                details: error.error.details(),
                recovery: error.error.recovery(),
            }))
        }
    }
}

fn dynamic_failure(failure: DynamicCommandFailure) -> crate::Result<HandlerOutcome> {
    match failure {
        DynamicCommandFailure::Framework(FrameworkError::CapabilityDenied { .. }) => {
            Err(FrameworkError::Handler(
                "result-aware handler returned legacy capability denial".to_string(),
            ))
        }
        DynamicCommandFailure::Framework(error) => Err(error),
        DynamicCommandFailure::Application(error) => {
            Ok(HandlerOutcome::ApplicationError(RawApplicationError {
                code: error.code,
                message: error.message,
                details: error.details,
                recovery: error.recovery,
            }))
        }
    }
}

fn application_success<O: ApplicationOutput>(output: O) -> crate::Result<HandlerOutcome> {
    let success = output.into_success();
    let value = serde_json::to_value(success.value).map_err(|_| {
        FrameworkError::ResultContractViolation {
            boundary: ResultContractBoundary::Success,
            reason: ResultContractReason::SerializationFailed,
        }
    })?;
    Ok(HandlerOutcome::ApplicationSuccess {
        value,
        grants: success.resources.grants,
        listings: success.resources.listings,
    })
}

// Implementations are written out below because each closure shape has a
// different call expression; the public marker family remains RFC 0012's.

impl<H, Fut, O, E, S> sealed::ResultDialect<ContextOnly<ApplicationOutputResult<O, E, S>>> for H
where
    H: Fn(CommandContext) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = ApplicationOutputResult<O, E, S>> + Send,
    O: ApplicationOutput,
    E: ApplicationError,
    S: ApplicationErrorSet<E>,
{
}

impl<H, Fut, O, E, S> ApplicationResultDialect<ContextOnly<ApplicationOutputResult<O, E, S>>> for H
where
    H: Fn(CommandContext) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = ApplicationOutputResult<O, E, S>> + Send,
    O: ApplicationOutput,
    E: ApplicationError,
    S: ApplicationErrorSet<E>,
{
    fn into_result_registration(self) -> ResultHandlerRegistration {
        typed_registration::<O, E, S>(
            Arc::new(TypedResultHandler {
                handler: self,
                _marker: PhantomData::<fn() -> ContextOnly<ApplicationOutputResult<O, E, S>>>,
            }),
            Vec::new(),
            None,
        )
    }
}

#[async_trait]
impl<H, Fut, O, E, S> ErasedCommandHandler
    for TypedResultHandler<ContextOnly<ApplicationOutputResult<O, E, S>>, H>
where
    H: Fn(CommandContext) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = ApplicationOutputResult<O, E, S>> + Send,
    O: ApplicationOutput,
    E: ApplicationError,
    S: ApplicationErrorSet<E>,
{
    async fn call(&self, context: CommandContext) -> crate::Result<HandlerOutcome> {
        match (self.handler)(context).await {
            Ok(output) => application_success(output),
            Err(error) => typed_failure(error),
        }
    }
}

impl<H, A, Fut, O, E, S> sealed::ResultDialect<ContextAndArgs<A, ApplicationOutputResult<O, E, S>>>
    for H
where
    H: Fn(CommandContext, A) -> Fut + Send + Sync + 'static,
    A: DeserializeOwned + JsonSchema + Send + Sync + 'static,
    Fut: Future<Output = ApplicationOutputResult<O, E, S>> + Send,
    O: ApplicationOutput,
    E: ApplicationError,
    S: ApplicationErrorSet<E>,
{
}

impl<H, A, Fut, O, E, S>
    ApplicationResultDialect<ContextAndArgs<A, ApplicationOutputResult<O, E, S>>> for H
where
    H: Fn(CommandContext, A) -> Fut + Send + Sync + 'static,
    A: DeserializeOwned + JsonSchema + Send + Sync + 'static,
    Fut: Future<Output = ApplicationOutputResult<O, E, S>> + Send,
    O: ApplicationOutput,
    E: ApplicationError,
    S: ApplicationErrorSet<E>,
{
    fn into_result_registration(self) -> ResultHandlerRegistration {
        typed_registration::<O, E, S>(
            Arc::new(TypedResultHandler {
                handler: self,
                _marker: PhantomData::<fn() -> ContextAndArgs<A, ApplicationOutputResult<O, E, S>>>,
            }),
            Vec::new(),
            Some(crate::argument_schemas::derived_argument_schema::<A>()),
        )
    }
}

#[async_trait]
impl<H, A, Fut, O, E, S> ErasedCommandHandler
    for TypedResultHandler<ContextAndArgs<A, ApplicationOutputResult<O, E, S>>, H>
where
    H: Fn(CommandContext, A) -> Fut + Send + Sync + 'static,
    A: DeserializeOwned + JsonSchema + Send + Sync + 'static,
    Fut: Future<Output = ApplicationOutputResult<O, E, S>> + Send,
    O: ApplicationOutput,
    E: ApplicationError,
    S: ApplicationErrorSet<E>,
{
    async fn call(&self, context: CommandContext) -> crate::Result<HandlerOutcome> {
        let args = if context.checked_argument_contract() {
            crate::argument_schemas::extract_checked::<A>(&context)?
        } else {
            A::from_command_args(&context)?
        };
        match (self.handler)(context, args).await {
            Ok(output) => application_success(output),
            Err(error) => typed_failure(error),
        }
    }
}

impl<H, P, Fut, O, E, S> sealed::ResultDialect<WithResources<P, ApplicationOutputResult<O, E, S>>>
    for H
where
    H: Fn(P, CommandContext) -> Fut + Send + Sync + 'static,
    P: ResourceParams,
    Fut: Future<Output = ApplicationOutputResult<O, E, S>> + Send,
    O: ApplicationOutput,
    E: ApplicationError,
    S: ApplicationErrorSet<E>,
{
}

impl<H, P, Fut, O, E, S>
    ApplicationResultDialect<WithResources<P, ApplicationOutputResult<O, E, S>>> for H
where
    H: Fn(P, CommandContext) -> Fut + Send + Sync + 'static,
    P: ResourceParams,
    Fut: Future<Output = ApplicationOutputResult<O, E, S>> + Send,
    O: ApplicationOutput,
    E: ApplicationError,
    S: ApplicationErrorSet<E>,
{
    fn into_result_registration(self) -> ResultHandlerRegistration {
        typed_registration::<O, E, S>(
            Arc::new(TypedResultHandler {
                handler: self,
                _marker: PhantomData::<fn() -> WithResources<P, ApplicationOutputResult<O, E, S>>>,
            }),
            P::resource_uses(),
            None,
        )
    }
}

#[async_trait]
impl<H, P, Fut, O, E, S> ErasedCommandHandler
    for TypedResultHandler<WithResources<P, ApplicationOutputResult<O, E, S>>, H>
where
    H: Fn(P, CommandContext) -> Fut + Send + Sync + 'static,
    P: ResourceParams,
    Fut: Future<Output = ApplicationOutputResult<O, E, S>> + Send,
    O: ApplicationOutput,
    E: ApplicationError,
    S: ApplicationErrorSet<E>,
{
    async fn call(&self, context: CommandContext) -> crate::Result<HandlerOutcome> {
        let params = P::extract(&context)?;
        match (self.handler)(params, context).await {
            Ok(output) => application_success(output),
            Err(error) => typed_failure(error),
        }
    }
}

impl<H, P, A, Fut, O, E, S>
    sealed::ResultDialect<WithResourcesAndArgs<P, A, ApplicationOutputResult<O, E, S>>> for H
where
    H: Fn(P, CommandContext, A) -> Fut + Send + Sync + 'static,
    P: ResourceParams,
    A: DeserializeOwned + JsonSchema + Send + Sync + 'static,
    Fut: Future<Output = ApplicationOutputResult<O, E, S>> + Send,
    O: ApplicationOutput,
    E: ApplicationError,
    S: ApplicationErrorSet<E>,
{
}

impl<H, P, A, Fut, O, E, S>
    ApplicationResultDialect<WithResourcesAndArgs<P, A, ApplicationOutputResult<O, E, S>>> for H
where
    H: Fn(P, CommandContext, A) -> Fut + Send + Sync + 'static,
    P: ResourceParams,
    A: DeserializeOwned + JsonSchema + Send + Sync + 'static,
    Fut: Future<Output = ApplicationOutputResult<O, E, S>> + Send,
    O: ApplicationOutput,
    E: ApplicationError,
    S: ApplicationErrorSet<E>,
{
    fn into_result_registration(self) -> ResultHandlerRegistration {
        typed_registration::<O, E, S>(
            Arc::new(TypedResultHandler {
                handler: self,
                _marker: PhantomData::<
                    fn() -> WithResourcesAndArgs<P, A, ApplicationOutputResult<O, E, S>>,
                >,
            }),
            P::resource_uses(),
            Some(crate::argument_schemas::derived_argument_schema::<A>()),
        )
    }
}

#[async_trait]
impl<H, P, A, Fut, O, E, S> ErasedCommandHandler
    for TypedResultHandler<WithResourcesAndArgs<P, A, ApplicationOutputResult<O, E, S>>, H>
where
    H: Fn(P, CommandContext, A) -> Fut + Send + Sync + 'static,
    P: ResourceParams,
    A: DeserializeOwned + JsonSchema + Send + Sync + 'static,
    Fut: Future<Output = ApplicationOutputResult<O, E, S>> + Send,
    O: ApplicationOutput,
    E: ApplicationError,
    S: ApplicationErrorSet<E>,
{
    async fn call(&self, context: CommandContext) -> crate::Result<HandlerOutcome> {
        let params = P::extract(&context)?;
        let args = if context.checked_argument_contract() {
            crate::argument_schemas::extract_checked::<A>(&context)?
        } else {
            A::from_command_args(&context)?
        };
        match (self.handler)(params, context, args).await {
            Ok(output) => application_success(output),
            Err(error) => typed_failure(error),
        }
    }
}

// Dynamic dialects use the same four marker shapes and require Value outputs.
impl<H, Fut, O> sealed::DynamicDialect<ContextOnly<DynamicApplicationResult<O>>> for H
where
    H: Fn(CommandContext) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = DynamicApplicationResult<O>> + Send,
    O: ApplicationOutput<Value = Value>,
{
}

impl<H, Fut, O> DynamicApplicationDialect<ContextOnly<DynamicApplicationResult<O>>> for H
where
    H: Fn(CommandContext) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = DynamicApplicationResult<O>> + Send,
    O: ApplicationOutput<Value = Value>,
{
    fn into_dynamic_registration(self) -> DynamicHandlerRegistration {
        dynamic_registration::<O>(
            Arc::new(DynamicResultHandler {
                handler: self,
                _marker: PhantomData::<fn() -> ContextOnly<DynamicApplicationResult<O>>>,
            }),
            Vec::new(),
        )
    }
}

#[async_trait]
impl<H, Fut, O> ErasedCommandHandler
    for DynamicResultHandler<ContextOnly<DynamicApplicationResult<O>>, H>
where
    H: Fn(CommandContext) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = DynamicApplicationResult<O>> + Send,
    O: ApplicationOutput<Value = Value>,
{
    async fn call(&self, context: CommandContext) -> crate::Result<HandlerOutcome> {
        match (self.handler)(context).await {
            Ok(output) => application_success(output),
            Err(error) => dynamic_failure(error),
        }
    }
}

impl<H, A, Fut, O> sealed::DynamicDialect<ContextAndArgs<A, DynamicApplicationResult<O>>> for H
where
    H: Fn(CommandContext, A) -> Fut + Send + Sync + 'static,
    A: DeserializeOwned + JsonSchema + Send + Sync + 'static,
    Fut: Future<Output = DynamicApplicationResult<O>> + Send,
    O: ApplicationOutput<Value = Value>,
{
}

impl<H, A, Fut, O> DynamicApplicationDialect<ContextAndArgs<A, DynamicApplicationResult<O>>> for H
where
    H: Fn(CommandContext, A) -> Fut + Send + Sync + 'static,
    A: DeserializeOwned + JsonSchema + Send + Sync + 'static,
    Fut: Future<Output = DynamicApplicationResult<O>> + Send,
    O: ApplicationOutput<Value = Value>,
{
    fn into_dynamic_registration(self) -> DynamicHandlerRegistration {
        dynamic_registration::<O>(
            Arc::new(DynamicResultHandler {
                handler: self,
                _marker: PhantomData::<fn() -> ContextAndArgs<A, DynamicApplicationResult<O>>>,
            }),
            Vec::new(),
        )
    }
}

#[async_trait]
impl<H, A, Fut, O> ErasedCommandHandler
    for DynamicResultHandler<ContextAndArgs<A, DynamicApplicationResult<O>>, H>
where
    H: Fn(CommandContext, A) -> Fut + Send + Sync + 'static,
    A: DeserializeOwned + JsonSchema + Send + Sync + 'static,
    Fut: Future<Output = DynamicApplicationResult<O>> + Send,
    O: ApplicationOutput<Value = Value>,
{
    async fn call(&self, context: CommandContext) -> crate::Result<HandlerOutcome> {
        let args = A::from_command_args(&context)?;
        match (self.handler)(context, args).await {
            Ok(output) => application_success(output),
            Err(error) => dynamic_failure(error),
        }
    }
}

// Resource-bearing dynamic handlers follow the typed implementations.
impl<H, P, Fut, O> sealed::DynamicDialect<WithResources<P, DynamicApplicationResult<O>>> for H
where
    H: Fn(P, CommandContext) -> Fut + Send + Sync + 'static,
    P: ResourceParams,
    Fut: Future<Output = DynamicApplicationResult<O>> + Send,
    O: ApplicationOutput<Value = Value>,
{
}

impl<H, P, Fut, O> DynamicApplicationDialect<WithResources<P, DynamicApplicationResult<O>>> for H
where
    H: Fn(P, CommandContext) -> Fut + Send + Sync + 'static,
    P: ResourceParams,
    Fut: Future<Output = DynamicApplicationResult<O>> + Send,
    O: ApplicationOutput<Value = Value>,
{
    fn into_dynamic_registration(self) -> DynamicHandlerRegistration {
        dynamic_registration::<O>(
            Arc::new(DynamicResultHandler {
                handler: self,
                _marker: PhantomData::<fn() -> WithResources<P, DynamicApplicationResult<O>>>,
            }),
            P::resource_uses(),
        )
    }
}

#[async_trait]
impl<H, P, Fut, O> ErasedCommandHandler
    for DynamicResultHandler<WithResources<P, DynamicApplicationResult<O>>, H>
where
    H: Fn(P, CommandContext) -> Fut + Send + Sync + 'static,
    P: ResourceParams,
    Fut: Future<Output = DynamicApplicationResult<O>> + Send,
    O: ApplicationOutput<Value = Value>,
{
    async fn call(&self, context: CommandContext) -> crate::Result<HandlerOutcome> {
        let params = P::extract(&context)?;
        match (self.handler)(params, context).await {
            Ok(output) => application_success(output),
            Err(error) => dynamic_failure(error),
        }
    }
}

impl<H, P, A, Fut, O>
    sealed::DynamicDialect<WithResourcesAndArgs<P, A, DynamicApplicationResult<O>>> for H
where
    H: Fn(P, CommandContext, A) -> Fut + Send + Sync + 'static,
    P: ResourceParams,
    A: DeserializeOwned + JsonSchema + Send + Sync + 'static,
    Fut: Future<Output = DynamicApplicationResult<O>> + Send,
    O: ApplicationOutput<Value = Value>,
{
}

impl<H, P, A, Fut, O>
    DynamicApplicationDialect<WithResourcesAndArgs<P, A, DynamicApplicationResult<O>>> for H
where
    H: Fn(P, CommandContext, A) -> Fut + Send + Sync + 'static,
    P: ResourceParams,
    A: DeserializeOwned + JsonSchema + Send + Sync + 'static,
    Fut: Future<Output = DynamicApplicationResult<O>> + Send,
    O: ApplicationOutput<Value = Value>,
{
    fn into_dynamic_registration(self) -> DynamicHandlerRegistration {
        dynamic_registration::<O>(
            Arc::new(DynamicResultHandler {
                handler: self,
                _marker: PhantomData::<
                    fn() -> WithResourcesAndArgs<P, A, DynamicApplicationResult<O>>,
                >,
            }),
            P::resource_uses(),
        )
    }
}

#[async_trait]
impl<H, P, A, Fut, O> ErasedCommandHandler
    for DynamicResultHandler<WithResourcesAndArgs<P, A, DynamicApplicationResult<O>>, H>
where
    H: Fn(P, CommandContext, A) -> Fut + Send + Sync + 'static,
    P: ResourceParams,
    A: DeserializeOwned + JsonSchema + Send + Sync + 'static,
    Fut: Future<Output = DynamicApplicationResult<O>> + Send,
    O: ApplicationOutput<Value = Value>,
{
    async fn call(&self, context: CommandContext) -> crate::Result<HandlerOutcome> {
        let params = P::extract(&context)?;
        let args = A::from_command_args(&context)?;
        match (self.handler)(params, context, args).await {
            Ok(output) => application_success(output),
            Err(error) => dynamic_failure(error),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum ResultContractBoundary {
    Success,
    ApplicationError,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum ResultContractReason {
    SerializationFailed,
    SchemaMismatch,
    UndeclaredCode,
    InvalidMessage,
    InvalidDetails,
    UndeclaredRecovery,
    InvalidRecoverySelection,
}

pub(crate) fn compile_contract(contract: &mut ApplicationResultContract) -> crate::Result<()> {
    canonicalize_schema(&mut contract.success_schema, false)?;
    canonicalize_error_specs(&mut contract.errors)?;
    for error in &mut contract.errors {
        close_error_detail_objects(&mut error.details_schema);
        canonicalize_schema(&mut error.details_schema, false)?;
        validate_error_spec(error)?;
    }
    Ok(())
}

fn close_error_detail_objects(schema: &mut Value) {
    let Some(object) = schema.as_object_mut() else {
        return;
    };
    if !object.contains_key("type")
        && (object.contains_key("properties") || object.contains_key("required"))
    {
        object.insert("type".to_string(), Value::String("object".to_string()));
    }
    let describes_object = match object.get("type") {
        Some(Value::String(kind)) => kind == "object",
        Some(Value::Array(kinds)) => kinds.iter().any(|kind| kind == "object"),
        _ => object.contains_key("properties") || object.contains_key("required"),
    };
    if describes_object && !object.contains_key("additionalProperties") {
        object.insert("additionalProperties".to_string(), Value::Bool(false));
    }
    if let Some(items) = object.get_mut("items") {
        close_error_detail_objects(items);
    }
    if let Some(properties) = object.get_mut("properties").and_then(Value::as_object_mut) {
        for property in properties.values_mut() {
            close_error_detail_objects(property);
        }
    }
    if let Some(additional) = object.get_mut("additionalProperties")
        && !additional.is_boolean()
    {
        close_error_detail_objects(additional);
    }
    if let Some(branches) = object.get_mut("oneOf").and_then(Value::as_array_mut) {
        for branch in branches {
            close_error_detail_objects(branch);
        }
    }
    if let Some(definitions) = object.get_mut("$defs").and_then(Value::as_object_mut) {
        for definition in definitions.values_mut() {
            close_error_detail_objects(definition);
        }
    }
}

pub(crate) fn compose_error_specs(
    declarations: &[ApplicationErrorDecl],
    uses: &[ApplicationErrorUse],
    capability_recoveries: impl Fn(&str) -> Option<Vec<String>>,
) -> crate::Result<Vec<ApplicationErrorSpec>> {
    let mut identities = BTreeMap::new();
    for declaration in declarations {
        validate_error_decl(declaration)?;
        if identities
            .insert(declaration.code.clone(), declaration)
            .is_some()
        {
            return Err(build_error(format!(
                "application error `{}` is declared more than once",
                declaration.code
            )));
        }
    }
    let mut seen = BTreeSet::new();
    let mut specs = Vec::new();
    for usage in uses {
        if !seen.insert(usage.code.clone()) {
            return Err(build_error(format!(
                "application error `{}` is used more than once by one command",
                usage.code
            )));
        }
        let declaration = identities.get(&usage.code).ok_or_else(|| {
            build_error(format!(
                "application error use `{}` has no matching declaration",
                usage.code
            ))
        })?;
        let mut recoveries = usage.recoveries.clone();
        if let Some(capability) = &usage.capability {
            if !usage.recoveries.is_empty()
                || usage.recovery_cardinality != RecoveryCardinality::Any
            {
                return Err(build_error(format!(
                    "capability-bound application error `{}` cannot author recovery entries or cardinality",
                    usage.code
                )));
            }
            let providers = capability_recoveries(capability).ok_or_else(|| {
                build_error(format!(
                    "application error `{}` binds unresolved capability `{capability}`",
                    usage.code
                ))
            })?;
            recoveries = providers
                .into_iter()
                .map(|operation_id| ApplicationRecoveryDecl::Operation { operation_id })
                .collect();
        }
        let spec = ApplicationErrorSpec {
            code: declaration.code.clone(),
            summary: declaration.summary.clone(),
            message: declaration.message.clone(),
            details_schema: declaration.details_schema.clone(),
            capability: usage.capability.clone(),
            recoveries,
            recovery_cardinality: usage.recovery_cardinality,
        };
        validate_error_spec(&spec)?;
        specs.push(spec);
    }
    specs.sort_by(|left, right| left.code.cmp(&right.code));
    Ok(specs)
}

fn canonicalize_error_specs(errors: &mut [ApplicationErrorSpec]) -> crate::Result<()> {
    errors.sort_by(|left, right| left.code.cmp(&right.code));
    for pair in errors.windows(2) {
        if pair[0].code == pair[1].code {
            return Err(build_error(format!(
                "application error `{}` appears more than once",
                pair[0].code
            )));
        }
    }
    Ok(())
}

fn validate_error_decl(decl: &ApplicationErrorDecl) -> crate::Result<()> {
    validate_code(&decl.code, "application error")?;
    validate_public_text(&decl.summary, "application error summary")?;
    validate_message_decl(&decl.message)?;
    let mut schema = decl.details_schema.clone();
    canonicalize_schema(&mut schema, false)
}

fn validate_error_spec(spec: &ApplicationErrorSpec) -> crate::Result<()> {
    validate_code(&spec.code, "application error")?;
    validate_public_text(&spec.summary, "application error summary")?;
    validate_message_decl(&spec.message)?;
    let mut keys = BTreeSet::new();
    for recovery in &spec.recoveries {
        let key = match recovery {
            ApplicationRecoveryDecl::Operation { operation_id } => {
                if operation_id.trim().is_empty() {
                    return Err(build_error("application recovery operation is empty"));
                }
                format!("operation:{operation_id}")
            }
            ApplicationRecoveryDecl::Action(action) => {
                validate_code(&action.code, "application recovery action")?;
                validate_public_text(&action.summary, "application recovery summary")?;
                format!("action:{}", action.code)
            }
        };
        if !keys.insert(key.clone()) {
            return Err(build_error(format!(
                "application recovery `{key}` is declared more than once"
            )));
        }
    }
    Ok(())
}

fn validate_message_decl(message: &ApplicationMessageDecl) -> crate::Result<()> {
    if let ApplicationMessageDecl::RuntimeBounded { max_scalar_values } = message
        && (*max_scalar_values == 0 || usize::from(*max_scalar_values) > MAX_PUBLIC_SCALARS)
    {
        return Err(build_error(format!(
            "runtime application message bound must be between 1 and {MAX_PUBLIC_SCALARS}"
        )));
    }
    Ok(())
}

pub(crate) fn validate_application_success(
    contract: &ApplicationResultContract,
    value: &Value,
) -> crate::Result<()> {
    if value_matches_schema(value, &contract.success_schema, &contract.success_schema) {
        Ok(())
    } else {
        Err(FrameworkError::ResultContractViolation {
            boundary: ResultContractBoundary::Success,
            reason: ResultContractReason::SchemaMismatch,
        })
    }
}

pub(crate) fn validate_application_error(
    contract: &ApplicationResultContract,
    raw: RawApplicationError,
) -> crate::Result<ApplicationErrorBody> {
    let spec = contract
        .errors
        .iter()
        .find(|error| error.code == raw.code)
        .ok_or(FrameworkError::ResultContractViolation {
            boundary: ResultContractBoundary::ApplicationError,
            reason: ResultContractReason::UndeclaredCode,
        })?;
    if !value_matches_schema(&raw.details, &spec.details_schema, &spec.details_schema) {
        return Err(FrameworkError::ResultContractViolation {
            boundary: ResultContractBoundary::ApplicationError,
            reason: ResultContractReason::InvalidDetails,
        });
    }
    let message = match (&spec.message, raw.message) {
        (ApplicationMessageDecl::DeclarationSummary, None) => spec.summary.clone(),
        (ApplicationMessageDecl::DeclarationSummary, Some(_))
        | (ApplicationMessageDecl::RuntimeBounded { .. }, None) => {
            return Err(FrameworkError::ResultContractViolation {
                boundary: ResultContractBoundary::ApplicationError,
                reason: ResultContractReason::InvalidMessage,
            });
        }
        (ApplicationMessageDecl::RuntimeBounded { .. }, Some(message)) if message.is_empty() => {
            return Err(FrameworkError::ResultContractViolation {
                boundary: ResultContractBoundary::ApplicationError,
                reason: ResultContractReason::InvalidMessage,
            });
        }
        (ApplicationMessageDecl::RuntimeBounded { max_scalar_values }, Some(message)) => {
            encode_public_text(&message, usize::from(*max_scalar_values))
        }
    };
    let recoveries = select_recoveries(spec, raw.recovery)?;
    Ok(ApplicationErrorBody {
        code: spec.code.clone(),
        message,
        details: raw.details,
        recoveries,
    })
}

fn select_recoveries(
    spec: &ApplicationErrorSpec,
    selection: ApplicationRecoverySelection,
) -> crate::Result<Vec<ApplicationRecovery>> {
    let selected = match selection {
        ApplicationRecoverySelection::Declared => spec.recoveries.clone(),
        ApplicationRecoverySelection::None => Vec::new(),
        ApplicationRecoverySelection::Only(keys) => {
            let mut wanted = BTreeSet::new();
            for key in keys {
                let key = match key {
                    ApplicationRecoveryKey::Operation(value) => format!("operation:{value}"),
                    ApplicationRecoveryKey::Action(value) => format!("action:{value}"),
                };
                if !wanted.insert(key) {
                    return Err(FrameworkError::ResultContractViolation {
                        boundary: ResultContractBoundary::ApplicationError,
                        reason: ResultContractReason::InvalidRecoverySelection,
                    });
                }
            }
            let selected = spec
                .recoveries
                .iter()
                .filter(|recovery| {
                    wanted.contains(&match recovery {
                        ApplicationRecoveryDecl::Operation { operation_id } => {
                            format!("operation:{operation_id}")
                        }
                        ApplicationRecoveryDecl::Action(action) => {
                            format!("action:{}", action.code)
                        }
                    })
                })
                .cloned()
                .collect::<Vec<_>>();
            if selected.len() != wanted.len() {
                return Err(FrameworkError::ResultContractViolation {
                    boundary: ResultContractBoundary::ApplicationError,
                    reason: ResultContractReason::UndeclaredRecovery,
                });
            }
            selected
        }
    };
    if spec.recovery_cardinality == RecoveryCardinality::AtMostOne && selected.len() > 1 {
        return Err(FrameworkError::ResultContractViolation {
            boundary: ResultContractBoundary::ApplicationError,
            reason: ResultContractReason::InvalidRecoverySelection,
        });
    }
    Ok(selected
        .into_iter()
        .map(|recovery| match recovery {
            ApplicationRecoveryDecl::Operation { operation_id } => {
                ApplicationRecovery::Operation { operation_id }
            }
            ApplicationRecoveryDecl::Action(action) => ApplicationRecovery::Action {
                code: action.code,
                summary: action.summary,
            },
        })
        .collect())
}

pub(crate) fn canonicalize_schema(schema: &mut Value, typed: bool) -> crate::Result<()> {
    let root = schema.as_object_mut().ok_or_else(|| {
        build_error("result schema root must be an object; boolean schemas are unsupported")
    })?;
    if let Some(marker) = root.remove("$schema")
        && marker != Value::String(JSON_SCHEMA_DIALECT.to_string())
    {
        return Err(build_error(
            "result schema `$schema` must name JSON Schema draft 2020-12",
        ));
    }
    if typed {
        normalize_typed_schema(schema);
    }
    canonicalize_nullable_types(schema);
    validate_schema_node(schema, true)?;
    validate_local_definitions(schema)
}

fn canonicalize_nullable_types(value: &mut Value) {
    match value {
        Value::Object(object) => {
            if let Some(Value::Array(kinds)) = object.get_mut("type") {
                kinds.sort_by_key(|kind| kind == "null");
            }
            for (key, nested) in object {
                if !matches!(key.as_str(), "const" | "enum") {
                    canonicalize_nullable_types(nested);
                }
            }
        }
        Value::Array(values) => {
            for value in values {
                canonicalize_nullable_types(value);
            }
        }
        _ => {}
    }
}

fn validate_schema_node(schema: &Value, root: bool) -> crate::Result<()> {
    let object = schema.as_object().ok_or_else(|| {
        build_error("result schemas must use object schemas; boolean schemas are unsupported")
    })?;
    const ALLOWED: &[&str] = &[
        "title",
        "description",
        "type",
        "const",
        "enum",
        "minLength",
        "items",
        "minItems",
        "properties",
        "required",
        "additionalProperties",
        "oneOf",
        "$defs",
        "$ref",
    ];
    for key in object.keys() {
        if !ALLOWED.contains(&key.as_str()) {
            let location = if root { "root" } else { "nested schema" };
            return Err(build_error(format!(
                "unsupported result schema keyword `{key}` at {location}"
            )));
        }
    }
    if !root && object.contains_key("$defs") {
        return Err(build_error(
            "result schema `$defs` is supported only at the schema root",
        ));
    }
    for annotation in ["title", "description"] {
        if let Some(value) = object.get(annotation)
            && !value.is_string()
        {
            return Err(build_error(format!(
                "result schema `{annotation}` must be a string"
            )));
        }
    }
    if let Some(reference) = object.get("$ref") {
        let reference = reference
            .as_str()
            .ok_or_else(|| build_error("result schema `$ref` must be a string"))?;
        if !reference.starts_with("#/$defs/") || reference[8..].contains('/') {
            return Err(build_error(format!(
                "result schema reference `{reference}` is not a local definition reference"
            )));
        }
    }
    if let Some(kind) = object.get("type") {
        validate_schema_type(kind)?;
    }
    if let Some(values) = object.get("enum") {
        let values = values
            .as_array()
            .ok_or_else(|| build_error("result schema `enum` must be an array"))?;
        if values.is_empty() {
            return Err(build_error("result schema `enum` cannot be empty"));
        }
        let kind = json_kind(&values[0]);
        if values.iter().any(|value| json_kind(value) != kind) {
            return Err(build_error("result schema `enum` must be homogeneous"));
        }
        for value in values {
            validate_schema_literal(value)?;
        }
    }
    if let Some(value) = object.get("const") {
        validate_schema_literal(value)?;
    }
    if let Some(value) = object.get("minLength")
        && value.as_u64().is_none()
    {
        return Err(build_error("result schema `minLength` must be an integer"));
    }
    if let Some(value) = object.get("minItems")
        && value.as_u64().is_none()
    {
        return Err(build_error("result schema `minItems` must be an integer"));
    }
    if let Some(items) = object.get("items") {
        validate_schema_node(items, false)?;
    }
    if let Some(properties) = object.get("properties") {
        for schema in properties
            .as_object()
            .ok_or_else(|| build_error("result schema `properties` must be an object"))?
            .values()
        {
            validate_schema_node(schema, false)?;
        }
    }
    if let Some(required) = object.get("required") {
        let required = required
            .as_array()
            .ok_or_else(|| build_error("result schema `required` must be an array"))?;
        if required.iter().any(|value| value.as_str().is_none()) {
            return Err(build_error(
                "result schema `required` entries must be strings",
            ));
        }
        let mut names = BTreeSet::new();
        if required
            .iter()
            .filter_map(Value::as_str)
            .any(|name| !names.insert(name))
        {
            return Err(build_error(
                "result schema `required` entries must be unique",
            ));
        }
        if let Some(properties) = object.get("properties").and_then(Value::as_object)
            && required
                .iter()
                .filter_map(Value::as_str)
                .any(|name| !properties.contains_key(name))
        {
            return Err(build_error(
                "result schema `required` names a missing property",
            ));
        }
    }
    if let Some(additional) = object.get("additionalProperties")
        && !additional.is_boolean()
    {
        validate_schema_node(additional, false)?;
    }
    if let Some(one_of) = object.get("oneOf") {
        let one_of = one_of
            .as_array()
            .ok_or_else(|| build_error("result schema `oneOf` must be an array"))?;
        if one_of.is_empty() {
            return Err(build_error("result schema `oneOf` cannot be empty"));
        }
        for branch in one_of {
            validate_schema_node(branch, false)?;
        }
    }
    if let Some(definitions) = object.get("$defs") {
        for schema in definitions
            .as_object()
            .ok_or_else(|| build_error("result schema `$defs` must be an object"))?
            .values()
        {
            validate_schema_node(schema, false)?;
        }
    }
    Ok(())
}

fn validate_schema_literal(value: &Value) -> crate::Result<()> {
    match value {
        Value::Number(number) => {
            const MAX_EXACT_INTEGER: i128 = 9_007_199_254_740_991;
            let exact = number
                .as_i64()
                .map(|value| i128::from(value).abs() <= MAX_EXACT_INTEGER)
                .or_else(|| {
                    number
                        .as_u64()
                        .map(|value| i128::from(value) <= MAX_EXACT_INTEGER)
                })
                .unwrap_or_else(|| {
                    number.as_f64().is_some_and(|value| {
                        value.is_finite()
                            && (value.fract() != 0.0 || value.abs() <= MAX_EXACT_INTEGER as f64)
                    })
                });
            if !exact {
                return Err(build_error(
                    "result schema numeric literal is outside the exact I-JSON domain",
                ));
            }
        }
        Value::Array(values) => {
            for value in values {
                validate_schema_literal(value)?;
            }
        }
        Value::Object(values) => {
            for value in values.values() {
                validate_schema_literal(value)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn validate_schema_type(value: &Value) -> crate::Result<()> {
    let valid = |kind: &str| {
        matches!(
            kind,
            "null" | "boolean" | "object" | "array" | "number" | "integer" | "string"
        )
    };
    match value {
        Value::String(kind) if valid(kind) => Ok(()),
        Value::Array(kinds) if kinds.len() == 2 => {
            let kinds = kinds
                .iter()
                .map(Value::as_str)
                .collect::<Option<Vec<_>>>()
                .ok_or_else(|| build_error("result schema type array must contain strings"))?;
            if kinds.iter().filter(|kind| **kind == "null").count() == 1
                && kinds
                    .iter()
                    .filter(|kind| **kind != "null")
                    .all(|kind| valid(kind))
            {
                Ok(())
            } else {
                Err(build_error(
                    "result schema type array must contain one primitive and null",
                ))
            }
        }
        _ => Err(build_error("result schema has an unsupported `type` value")),
    }
}

fn validate_local_definitions(schema: &Value) -> crate::Result<()> {
    let definitions = schema
        .get("$defs")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let mut reachable = BTreeSet::new();
    let mut visiting = BTreeSet::new();
    visit_schema_refs(schema, &definitions, &mut reachable, &mut visiting)?;
    for name in definitions.keys() {
        if !reachable.contains(name) {
            return Err(build_error(format!(
                "result schema definition `{name}` is unreachable"
            )));
        }
    }
    Ok(())
}

fn visit_schema_refs(
    value: &Value,
    definitions: &Map<String, Value>,
    reachable: &mut BTreeSet<String>,
    visiting: &mut BTreeSet<String>,
) -> crate::Result<()> {
    match value {
        Value::Object(object) => {
            if let Some(reference) = object.get("$ref").and_then(Value::as_str) {
                let name = reference.trim_start_matches("#/$defs/");
                let target = definitions.get(name).ok_or_else(|| {
                    build_error(format!("result schema reference `{reference}` is dangling"))
                })?;
                if !visiting.insert(name.to_string()) {
                    return Err(build_error(format!(
                        "result schema reference cycle through `{name}`"
                    )));
                }
                reachable.insert(name.to_string());
                visit_schema_refs(target, definitions, reachable, visiting)?;
                visiting.remove(name);
            }
            for (key, nested) in object {
                if !matches!(key.as_str(), "$defs" | "const" | "enum") {
                    visit_schema_refs(nested, definitions, reachable, visiting)?;
                }
            }
        }
        Value::Array(values) => {
            for nested in values {
                visit_schema_refs(nested, definitions, reachable, visiting)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn normalize_typed_schema(value: &mut Value) {
    let definitions = value
        .get("$defs")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    normalize_typed_schema_at(value, &definitions);
}

fn normalize_typed_schema_at(value: &mut Value, definitions: &Map<String, Value>) {
    match value {
        Value::Object(object) => {
            if let Some(format) = object.get("format").and_then(Value::as_str)
                && let Some((storage_type, storage_minimum, storage_maximum)) =
                    rust_storage_shapes().get(format)
                && schema_type_matches_storage(object.get("type"), storage_type)
                && object.get("minimum") == storage_minimum.as_ref()
                && object.get("maximum") == storage_maximum.as_ref()
            {
                if storage_minimum.is_some() {
                    object.remove("minimum");
                }
                if storage_maximum.is_some() {
                    object.remove("maximum");
                }
                object.remove("format");
            }
            if let Some(any_of) = object.remove("anyOf") {
                if let Value::Array(branches) = &any_of
                    && branches.len() == 2
                    && branches
                        .iter()
                        .filter(|branch| branch.get("type") == Some(&json!("null")))
                        .count()
                        == 1
                    && branches
                        .iter()
                        .find(|branch| branch.get("type") != Some(&json!("null")))
                        .is_some_and(|branch| {
                            schema_is_provably_non_null(branch, definitions, &mut BTreeSet::new())
                        })
                {
                    object.insert("oneOf".to_string(), any_of);
                } else {
                    object.insert("anyOf".to_string(), any_of);
                }
            }
            if let Some(Value::Array(kinds)) = object.get_mut("type") {
                kinds.sort_by_key(|kind| kind == "null");
            }
            for (key, nested) in object {
                if !matches!(key.as_str(), "const" | "enum") {
                    normalize_typed_schema_at(nested, definitions);
                }
            }
        }
        Value::Array(values) => {
            for nested in values {
                normalize_typed_schema_at(nested, definitions);
            }
        }
        _ => {}
    }
}

fn schema_type_matches_storage(actual: Option<&Value>, storage_type: &Value) -> bool {
    actual == Some(storage_type)
        || actual.and_then(Value::as_array).is_some_and(|types| {
            types.len() == 2
                && types.iter().any(|kind| kind == storage_type)
                && types.iter().any(|kind| kind == "null")
        })
}

fn rust_storage_shapes() -> &'static RustStorageShapes {
    static SHAPES: OnceLock<RustStorageShapes> = OnceLock::new();
    SHAPES.get_or_init(|| {
        fn insert<T: JsonSchema>(shapes: &mut RustStorageShapes) {
            let generator = SchemaSettings::draft2020_12().into_generator();
            let schema = serde_json::to_value(generator.into_root_schema_for::<T>())
                .expect("Schemars primitive schema serializes as JSON");
            if let (Some(format), Some(storage_type)) = (
                schema.get("format").and_then(Value::as_str),
                schema.get("type").cloned(),
            ) {
                shapes.insert(
                    format.to_string(),
                    (
                        storage_type,
                        schema.get("minimum").cloned(),
                        schema.get("maximum").cloned(),
                    ),
                );
            }
        }

        let mut shapes = BTreeMap::new();
        insert::<i8>(&mut shapes);
        insert::<i16>(&mut shapes);
        insert::<i32>(&mut shapes);
        insert::<i64>(&mut shapes);
        insert::<i128>(&mut shapes);
        insert::<u8>(&mut shapes);
        insert::<u16>(&mut shapes);
        insert::<u32>(&mut shapes);
        insert::<u64>(&mut shapes);
        insert::<u128>(&mut shapes);
        insert::<f32>(&mut shapes);
        insert::<f64>(&mut shapes);
        shapes
    })
}

fn schema_is_provably_non_null(
    schema: &Value,
    definitions: &Map<String, Value>,
    visiting: &mut BTreeSet<String>,
) -> bool {
    let Some(object) = schema.as_object() else {
        return false;
    };
    if object.get("const").is_some_and(|value| !value.is_null()) {
        return true;
    }
    if object
        .get("enum")
        .and_then(Value::as_array)
        .is_some_and(|values| !values.is_empty() && values.iter().all(|value| !value.is_null()))
    {
        return true;
    }
    let own_assertion_is_non_null = match object.get("type") {
        Some(Value::String(kind)) => kind != "null",
        Some(Value::Array(kinds)) => !kinds.iter().any(|kind| kind == "null"),
        _ => false,
    };
    if own_assertion_is_non_null {
        return true;
    }
    if object
        .get("oneOf")
        .and_then(Value::as_array)
        .is_some_and(|branches| {
            !branches.is_empty()
                && branches
                    .iter()
                    .all(|branch| schema_is_provably_non_null(branch, definitions, visiting))
        })
    {
        return true;
    }
    let Some(reference) = object.get("$ref").and_then(Value::as_str) else {
        return false;
    };
    let Some(name) = reference.strip_prefix("#/$defs/") else {
        return false;
    };
    if name.contains('/') || !visiting.insert(name.to_string()) {
        return false;
    }
    let non_null = definitions
        .get(name)
        .is_some_and(|target| schema_is_provably_non_null(target, definitions, visiting));
    visiting.remove(name);
    non_null
}

fn value_matches_schema(value: &Value, schema: &Value, root: &Value) -> bool {
    let Some(object) = schema.as_object() else {
        return false;
    };
    if let Some(reference) = object.get("$ref").and_then(Value::as_str) {
        let name = reference.trim_start_matches("#/$defs/");
        if !root
            .get("$defs")
            .and_then(|defs| defs.get(name))
            .is_some_and(|schema| value_matches_schema(value, schema, root))
        {
            return false;
        }
    }
    if let Some(branches) = object.get("oneOf").and_then(Value::as_array)
        && branches
            .iter()
            .filter(|branch| value_matches_schema(value, branch, root))
            .count()
            != 1
    {
        return false;
    }
    if let Some(kind) = object.get("type")
        && !matches_schema_type(value, kind)
    {
        return false;
    }
    if let Some(expected) = object.get("const")
        && value != expected
    {
        return false;
    }
    if let Some(values) = object.get("enum").and_then(Value::as_array)
        && !values.contains(value)
    {
        return false;
    }
    if let (Some(text), Some(minimum)) = (
        value.as_str(),
        object.get("minLength").and_then(Value::as_u64),
    ) && text.chars().count() < minimum as usize
    {
        return false;
    }
    if let Some(items) = value.as_array() {
        if let Some(minimum) = object.get("minItems").and_then(Value::as_u64)
            && items.len() < minimum as usize
        {
            return false;
        }
        if let Some(item_schema) = object.get("items")
            && items
                .iter()
                .any(|item| !value_matches_schema(item, item_schema, root))
        {
            return false;
        }
    }
    if let Some(map) = value.as_object() {
        if let Some(required) = object.get("required").and_then(Value::as_array)
            && required
                .iter()
                .filter_map(Value::as_str)
                .any(|name| !map.contains_key(name))
        {
            return false;
        }
        let properties = object.get("properties").and_then(Value::as_object);
        if let Some(properties) = properties {
            for (name, property_schema) in properties {
                if let Some(value) = map.get(name)
                    && !value_matches_schema(value, property_schema, root)
                {
                    return false;
                }
            }
        }
        if let Some(additional) = object.get("additionalProperties") {
            for (name, value) in map {
                if properties.is_some_and(|properties| properties.contains_key(name)) {
                    continue;
                }
                match additional {
                    Value::Bool(false) => return false,
                    Value::Bool(true) => {}
                    schema if !value_matches_schema(value, schema, root) => return false,
                    _ => {}
                }
            }
        }
    }
    true
}

fn matches_schema_type(value: &Value, kind: &Value) -> bool {
    match kind {
        Value::String(kind) => matches_one_type(value, kind),
        Value::Array(kinds) => kinds
            .iter()
            .filter_map(Value::as_str)
            .any(|kind| matches_one_type(value, kind)),
        _ => false,
    }
}

fn matches_one_type(value: &Value, kind: &str) -> bool {
    match kind {
        "null" => value.is_null(),
        "boolean" => value.is_boolean(),
        "object" => value.is_object(),
        "array" => value.is_array(),
        "number" => value.is_number(),
        "integer" => {
            value.as_i64().is_some()
                || value.as_u64().is_some()
                || value
                    .as_f64()
                    .is_some_and(|value| value.is_finite() && value.fract() == 0.0)
        }
        "string" => value.is_string(),
        _ => false,
    }
}

fn closed_empty_object_schema() -> Value {
    json!({
        "type": "object",
        "properties": {},
        "additionalProperties": false,
    })
}

fn validate_code(code: &str, subject: &str) -> crate::Result<()> {
    let valid = !code.is_empty()
        && code.len() <= 128
        && code.bytes().enumerate().all(|(index, byte)| match byte {
            b'a'..=b'z' => true,
            b'0'..=b'9' => index > 0,
            b'_' => index > 0 && index + 1 < code.len(),
            _ => false,
        })
        && !code.contains("__");
    if valid {
        Ok(())
    } else {
        Err(build_error(format!(
            "{subject} code `{code}` must use lower snake case"
        )))
    }
}

fn validate_public_text(text: &str, subject: &str) -> crate::Result<()> {
    let count = text.chars().count();
    if text.trim().is_empty() || count > MAX_PUBLIC_SCALARS || text.chars().any(unsafe_scalar) {
        Err(build_error(format!(
            "{subject} must contain 1..={MAX_PUBLIC_SCALARS} display-safe scalars"
        )))
    } else {
        Ok(())
    }
}

fn unsafe_scalar(value: char) -> bool {
    matches!(value,
        '\u{0000}'..='\u{001F}'
        | '\u{007F}'..='\u{009F}'
        | '\u{061C}'
        | '\u{200E}'..='\u{200F}'
        | '\u{2028}'..='\u{202E}'
        | '\u{2060}'..='\u{206F}'
        | '\u{FEFF}'
    )
}

fn encode_public_text(text: &str, limit: usize) -> String {
    let mut output = String::new();
    let mut width = 0;
    let mut input = text.chars().peekable();
    while let Some(value) = input.next() {
        let chunk = match value {
            '"' => "\\\"".to_string(),
            '\\' => "\\\\".to_string(),
            '\u{0008}' => "\\b".to_string(),
            '\u{000C}' => "\\f".to_string(),
            '\n' => "\\n".to_string(),
            '\r' => "\\r".to_string(),
            '\t' => "\\t".to_string(),
            value if unsafe_scalar(value) => format!("\\u{:04X}", value as u32),
            value => value.to_string(),
        };
        let chunk_width = chunk.chars().count();
        let available = if input.peek().is_some() {
            limit.saturating_sub(1)
        } else {
            limit
        };
        if width + chunk_width > available {
            output.push('…');
            break;
        }
        output.push_str(&chunk);
        width += chunk_width;
    }
    output
}

fn json_kind(value: &Value) -> u8 {
    match value {
        Value::Null => 0,
        Value::Bool(_) => 1,
        Value::Number(_) => 2,
        Value::String(_) => 3,
        Value::Array(_) => 4,
        Value::Object(_) => 5,
    }
}

fn build_error(message: impl Into<String>) -> FrameworkError {
    FrameworkError::Build(message.into())
}
