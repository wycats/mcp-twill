//! First-class resources (RFC 0012): server-held resources with identity,
//! lifetime, and lifecycle edges derived from handler signatures. Handlers
//! require, grant, release, and enumerate resources through their types;
//! the framework derives the acquire/use/release graph at registration, so
//! the declaration cannot drift from the behavior. Resolution stays the
//! server author's job: a resolver answers resolved-or-refused, and the
//! framework knows — from the catalog — what to say when the answer is
//! refused.

use std::{
    any::Any, collections::BTreeMap, future::Future, marker::PhantomData, ops::Deref, pin::Pin,
    sync::Arc,
};

use async_trait::async_trait;
use serde_json::Value;

use crate::{
    CommandContext, CommandHandler, CommandOutput, FrameworkError, FromCommandArgs, InvocationPlan,
    ResolveResourceWithErrors, ResourceRef, ResourceResolutionFailure, Result,
};

/// Marker trait connecting a handler-side type to a declared resource.
/// The type is what the resolver produces; the name is what the catalog
/// declares.
pub trait Resource: Send + Sync + 'static {
    const NAME: &'static str;
}

/// A resolver's refusal: the resource exists as a concept but this
/// reference does not resolve (stale lease, foreign tab, expired session).
/// The detail is the resolver's prose; the recovery edges are the
/// catalog's, filled in by the framework.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceRefusal {
    pub detail: String,
}

impl ResourceRefusal {
    pub fn new(detail: impl Into<String>) -> Self {
        Self {
            detail: detail.into(),
        }
    }
}

/// Resolves a reference into a live resource value. The server author owns
/// resolution — the framework hands over the normalized reference and the
/// invocation plan, and receives the resource or a structured refusal. The
/// framework never sees a lease table.
pub trait ResolveResource<T: Resource>: Send + Sync + 'static {
    fn resolve(
        &self,
        reference: &str,
        plan: &InvocationPlan,
    ) -> impl Future<Output = std::result::Result<T, ResourceRefusal>> + Send;
}

/// Reads a resource's content for MCP `resources/read`. Binding a reader is
/// what turns grants into `resource_link` content parts: a link the server
/// cannot read is a dead link, so no reader means no links.
pub trait ReadResource<T: Resource>: Send + Sync + 'static {
    fn read(
        &self,
        id: &str,
    ) -> impl Future<Output = std::result::Result<Value, ResourceRefusal>> + Send;
}

type ResolveFuture<'a> = Pin<
    Box<
        dyn Future<
                Output = std::result::Result<Arc<dyn Any + Send + Sync>, ErasedResolutionFailure>,
            > + Send
            + 'a,
    >,
>;

pub(crate) enum ErasedResolutionFailure {
    Refused(ResourceRefusal),
    Application(crate::results::RawApplicationError),
    Framework(FrameworkError),
}

type ReadFuture<'a> =
    Pin<Box<dyn Future<Output = std::result::Result<Value, ResourceRefusal>> + Send + 'a>>;

pub(crate) trait ErasedResolver: Send + Sync {
    fn resolve_erased<'a>(
        &'a self,
        reference: &'a str,
        plan: &'a InvocationPlan,
    ) -> ResolveFuture<'a>;
}

pub(crate) struct ResolverAdapter<T, R> {
    resolver: R,
    _marker: PhantomData<fn() -> T>,
}

impl<T, R> ResolverAdapter<T, R> {
    pub(crate) fn new(resolver: R) -> Self {
        Self {
            resolver,
            _marker: PhantomData,
        }
    }
}

impl<T, R> ErasedResolver for ResolverAdapter<T, R>
where
    T: Resource,
    R: ResolveResource<T>,
{
    fn resolve_erased<'a>(
        &'a self,
        reference: &'a str,
        plan: &'a InvocationPlan,
    ) -> ResolveFuture<'a> {
        Box::pin(async move {
            let value = self
                .resolver
                .resolve(reference, plan)
                .await
                .map_err(ErasedResolutionFailure::Refused)?;
            Ok(Arc::new(value) as Arc<dyn Any + Send + Sync>)
        })
    }
}

pub(crate) struct ResolverWithErrorsAdapter<T, R> {
    resolver: R,
    _marker: PhantomData<fn() -> T>,
}

impl<T, R> ResolverWithErrorsAdapter<T, R> {
    pub(crate) fn new(resolver: R) -> Self {
        Self {
            resolver,
            _marker: PhantomData,
        }
    }
}

impl<T, R> ErasedResolver for ResolverWithErrorsAdapter<T, R>
where
    T: Resource,
    R: ResolveResourceWithErrors<T>,
{
    fn resolve_erased<'a>(
        &'a self,
        reference: &'a str,
        plan: &'a InvocationPlan,
    ) -> ResolveFuture<'a> {
        Box::pin(async move {
            match self.resolver.resolve(reference, plan).await {
                Ok(value) => Ok(Arc::new(value) as Arc<dyn Any + Send + Sync>),
                Err(ResourceResolutionFailure::Refused(refusal)) => {
                    Err(ErasedResolutionFailure::Refused(refusal))
                }
                Err(ResourceResolutionFailure::Application(error)) => match error.into_raw() {
                    Ok(error) => Err(ErasedResolutionFailure::Application(error)),
                    Err(error) => Err(ErasedResolutionFailure::Framework(error)),
                },
            }
        })
    }
}

pub(crate) trait ErasedReader: Send + Sync {
    fn read_erased<'a>(&'a self, id: &'a str) -> ReadFuture<'a>;
}

pub(crate) struct ReaderAdapter<T, R> {
    reader: R,
    _marker: PhantomData<fn() -> T>,
}

impl<T, R> ReaderAdapter<T, R> {
    pub(crate) fn new(reader: R) -> Self {
        Self {
            reader,
            _marker: PhantomData,
        }
    }
}

impl<T, R> ErasedReader for ReaderAdapter<T, R>
where
    T: Resource,
    R: ReadResource<T>,
{
    fn read_erased<'a>(&'a self, id: &'a str) -> ReadFuture<'a> {
        Box::pin(self.reader.read(id))
    }
}

/// The resources the framework resolved for one invocation, keyed by
/// resource name. Extractor parameters (`Res<T>`, `Release<T>`) downcast
/// from here inside the handler adapter.
#[derive(Clone, Default)]
pub struct ResolvedResources {
    values: BTreeMap<String, Arc<dyn Any + Send + Sync>>,
}

impl ResolvedResources {
    pub(crate) fn insert(&mut self, name: String, value: Arc<dyn Any + Send + Sync>) {
        self.values.insert(name, value);
    }

    pub fn get<T: Resource>(&self) -> Option<Arc<T>> {
        self.values.get(T::NAME)?.clone().downcast::<T>().ok()
    }
}

impl std::fmt::Debug for ResolvedResources {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResolvedResources")
            .field("resources", &self.values.keys().collect::<Vec<_>>())
            .finish()
    }
}

/// Resolved values are opaque; equality compares which resources were
/// resolved, which is the only fact the plan-level types can see.
impl PartialEq for ResolvedResources {
    fn eq(&self, other: &Self) -> bool {
        self.values.keys().eq(other.values.keys())
    }
}

/// A required resource, resolved before the handler runs. The value is
/// proof of resolution, not a string to re-validate.
pub struct Res<T: Resource>(Arc<T>);

impl<T: Resource> Deref for Res<T> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.0
    }
}

/// A required resource this handler releases. Resolution works exactly like
/// `Res<T>`; the parameter additionally declares the teardown edge. The
/// handler body still performs the actual teardown against its broker —
/// the framework cannot drop what it does not own.
pub struct Release<T: Resource>(Arc<T>);

impl<T: Resource> Deref for Release<T> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.0
    }
}

/// One resource relationship a handler signature declares.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResourceUse {
    pub resource: &'static str,
    pub released: bool,
}

/// A single extractor parameter: which resource it requires and how to pull
/// the resolved value out of the invocation.
pub trait ResourceParam: Sized + Send + Sync + 'static {
    fn resource_use() -> ResourceUse;
    fn is_optional_requirement() -> bool {
        false
    }
    fn extract(context: &CommandContext) -> Result<Self>;
}

fn unresolved(name: &str) -> FrameworkError {
    FrameworkError::Handler(format!(
        "resource `{name}` was not resolved before the handler ran"
    ))
}

impl<T: Resource> ResourceParam for Res<T> {
    fn resource_use() -> ResourceUse {
        ResourceUse {
            resource: T::NAME,
            released: false,
        }
    }

    fn extract(context: &CommandContext) -> Result<Self> {
        context
            .resources
            .get::<T>()
            .map(Res)
            .ok_or_else(|| unresolved(T::NAME))
    }
}

impl<T: Resource> ResourceParam for Release<T> {
    fn resource_use() -> ResourceUse {
        ResourceUse {
            resource: T::NAME,
            released: true,
        }
    }

    fn extract(context: &CommandContext) -> Result<Self> {
        context
            .resources
            .get::<T>()
            .map(Release)
            .ok_or_else(|| unresolved(T::NAME))
    }
}

impl<T: Resource> ResourceParam for Option<Res<T>> {
    fn resource_use() -> ResourceUse {
        <Res<T> as ResourceParam>::resource_use()
    }

    fn is_optional_requirement() -> bool {
        true
    }

    fn extract(context: &CommandContext) -> Result<Self> {
        Ok(context.resources.get::<T>().map(Res))
    }
}

/// The full resource-parameter position of a handler: one extractor or a
/// tuple of extractors.
pub trait ResourceParams: Sized + Send + Sync + 'static {
    fn resource_uses() -> Vec<ResourceUse>;
    fn optional_resources() -> Vec<&'static str> {
        Vec::new()
    }
    fn extract(context: &CommandContext) -> Result<Self>;
}

impl<P: ResourceParam> ResourceParams for P {
    fn resource_uses() -> Vec<ResourceUse> {
        vec![P::resource_use()]
    }

    fn optional_resources() -> Vec<&'static str> {
        P::is_optional_requirement()
            .then_some(P::resource_use().resource)
            .into_iter()
            .collect()
    }

    fn extract(context: &CommandContext) -> Result<Self> {
        P::extract(context)
    }
}

macro_rules! impl_resource_params_for_tuple {
    ($($param:ident),+) => {
        impl<$($param: ResourceParam),+> ResourceParams for ($($param,)+) {
            fn resource_uses() -> Vec<ResourceUse> {
                vec![$($param::resource_use()),+]
            }

            fn optional_resources() -> Vec<&'static str> {
                vec![$($param::is_optional_requirement().then_some($param::resource_use().resource)),+]
                    .into_iter()
                    .flatten()
                    .collect()
            }

            fn extract(context: &CommandContext) -> Result<Self> {
                Ok(($($param::extract(context)?,)+))
            }
        }
    };
}

impl_resource_params_for_tuple!(P1, P2);
impl_resource_params_for_tuple!(P1, P2, P3);
impl_resource_params_for_tuple!(P1, P2, P3, P4);
impl_resource_params_for_tuple!(P1, P2, P3, P4, P5);
impl_resource_params_for_tuple!(P1, P2, P3, P4, P5, P6);
impl_resource_params_for_tuple!(P1, P2, P3, P4, P5, P6, P7);
impl_resource_params_for_tuple!(P1, P2, P3, P4, P5, P6, P7, P8);

/// A minted reference this call grants. Attaching one to a `CommandOutput`
/// moves the output into a type that names the resource, so the grant edge
/// is readable from the handler's type at registration.
pub struct Grant<T: Resource> {
    id: String,
    _marker: PhantomData<fn() -> T>,
}

impl<T: Resource> Grant<T> {
    pub(crate) fn into_id(self) -> String {
        self.id
    }
}

impl<T: Resource> Grant<T> {
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            _marker: PhantomData,
        }
    }
}

/// The references this call enumerated — the recovery path for agents
/// whose context lost the ids.
pub struct Listing<T: Resource> {
    ids: Vec<String>,
    _marker: PhantomData<fn() -> T>,
}

impl<T: Resource> Listing<T> {
    pub fn new(ids: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self {
            ids: ids.into_iter().map(Into::into).collect(),
            _marker: PhantomData,
        }
    }

    pub(crate) fn into_ids(self) -> Vec<String> {
        self.ids
    }
}

/// A `CommandOutput` that grants a reference to `T`. Produced by
/// [`CommandOutput::grant`]; the type parameter is what lets registration
/// derive the grant edge statically.
pub struct Granted<T: Resource, O = CommandOutput> {
    output: O,
    grant: Grant<T>,
}

/// A `CommandOutput` that enumerates references to `T`. Produced by
/// [`CommandOutput::listing`].
pub struct Listed<T: Resource, O = CommandOutput> {
    output: O,
    listing: Listing<T>,
}

impl<T: Resource, O> Granted<T, O> {
    pub(crate) fn new(output: O, grant: Grant<T>) -> Self {
        Self { output, grant }
    }

    pub(crate) fn into_parts(self) -> (O, Grant<T>) {
        (self.output, self.grant)
    }
}

impl<T: Resource, O> Listed<T, O> {
    pub(crate) fn new(output: O, listing: Listing<T>) -> Self {
        Self { output, listing }
    }

    pub(crate) fn into_parts(self) -> (O, Listing<T>) {
        (self.output, self.listing)
    }
}

impl CommandOutput {
    /// Attaches a granted reference. The framework mints the URI from the
    /// declared template after the handler returns.
    pub fn grant<T: Resource>(self, grant: Grant<T>) -> Granted<T> {
        Granted::new(self, grant)
    }

    /// Attaches enumerated references. The framework mints URIs from the
    /// declared template after the handler returns.
    pub fn listing<T: Resource>(self, listing: Listing<T>) -> Listed<T> {
        Listed::new(self, listing)
    }
}

impl<T: Resource> ResourceOutput for Granted<T> {
    fn granted() -> Vec<&'static str> {
        vec![T::NAME]
    }

    fn enumerated() -> Vec<&'static str> {
        Vec::new()
    }

    fn into_command_output(self) -> CommandOutput {
        let (mut output, grant) = self.into_parts();
        output.grants.push(ResourceRef {
            resource: T::NAME.to_string(),
            id: grant.into_id(),
            uri: String::new(),
        });
        output
    }
}

impl<T: Resource> ResourceOutput for Listed<T> {
    fn granted() -> Vec<&'static str> {
        Vec::new()
    }

    fn enumerated() -> Vec<&'static str> {
        vec![T::NAME]
    }

    fn into_command_output(self) -> CommandOutput {
        let (mut output, listing) = self.into_parts();
        for id in listing.into_ids() {
            output.listings.push(ResourceRef {
                resource: T::NAME.to_string(),
                id,
                uri: String::new(),
            });
        }
        output
    }
}

/// A handler output the resource dialect understands: plain output, a
/// grant, or a listing. The static edge lists are read at registration;
/// the conversion happens per call.
pub trait ResourceOutput: Send + 'static {
    fn granted() -> Vec<&'static str>;
    fn enumerated() -> Vec<&'static str>;
    fn into_command_output(self) -> CommandOutput;
}

impl ResourceOutput for CommandOutput {
    fn granted() -> Vec<&'static str> {
        Vec::new()
    }

    fn enumerated() -> Vec<&'static str> {
        Vec::new()
    }

    fn into_command_output(self) -> CommandOutput {
        self
    }
}

/// Marker types disambiguating the supported handler shapes. Inferred,
/// never written by authors.
pub struct WithResources<P, O>(PhantomData<fn() -> (P, O)>);
pub struct WithResourcesAndArgs<P, A, O>(PhantomData<fn(P, A) -> O>);
pub struct ContextOnly<O>(PhantomData<fn() -> O>);
pub struct ContextAndArgs<A, O>(PhantomData<fn() -> (A, O)>);

/// A handler written in the resource dialect: its signature carries the
/// resource relationships, and registration reads them from the type.
pub trait ResourceDialect<M>: Send + Sync + 'static {
    fn resource_uses() -> Vec<ResourceUse>;
    fn optional_resources() -> Vec<&'static str> {
        Vec::new()
    }
    fn granted() -> Vec<&'static str>;
    fn enumerated() -> Vec<&'static str>;
    #[doc(hidden)]
    fn accepts_arguments() -> bool {
        false
    }
    fn into_command_handler(self) -> Arc<dyn CommandHandler>;
}

struct DialectHandler<M, H> {
    handler: H,
    _marker: PhantomData<fn() -> M>,
}

impl<F, P, Fut, O> ResourceDialect<WithResources<P, O>> for F
where
    F: Fn(P, CommandContext) -> Fut + Send + Sync + 'static,
    P: ResourceParams,
    Fut: Future<Output = Result<O>> + Send,
    O: ResourceOutput,
{
    fn resource_uses() -> Vec<ResourceUse> {
        P::resource_uses()
    }

    fn optional_resources() -> Vec<&'static str> {
        P::optional_resources()
    }

    fn granted() -> Vec<&'static str> {
        O::granted()
    }

    fn enumerated() -> Vec<&'static str> {
        O::enumerated()
    }

    fn accepts_arguments() -> bool {
        false
    }

    fn into_command_handler(self) -> Arc<dyn CommandHandler> {
        Arc::new(DialectHandler::<WithResources<P, O>, F> {
            handler: self,
            _marker: PhantomData,
        })
    }
}

#[async_trait]
impl<F, P, Fut, O> CommandHandler for DialectHandler<WithResources<P, O>, F>
where
    F: Fn(P, CommandContext) -> Fut + Send + Sync + 'static,
    P: ResourceParams,
    Fut: Future<Output = Result<O>> + Send,
    O: ResourceOutput,
{
    async fn call(&self, context: CommandContext) -> Result<CommandOutput> {
        let params = P::extract(&context)?;
        let output = (self.handler)(params, context).await?;
        Ok(output.into_command_output())
    }
}

impl<F, P, A, Fut, O> ResourceDialect<WithResourcesAndArgs<P, A, O>> for F
where
    F: Fn(P, CommandContext, A) -> Fut + Send + Sync + 'static,
    P: ResourceParams,
    A: FromCommandArgs + Send + Sync + 'static,
    Fut: Future<Output = Result<O>> + Send,
    O: ResourceOutput,
{
    fn resource_uses() -> Vec<ResourceUse> {
        P::resource_uses()
    }

    fn optional_resources() -> Vec<&'static str> {
        P::optional_resources()
    }

    fn granted() -> Vec<&'static str> {
        O::granted()
    }

    fn enumerated() -> Vec<&'static str> {
        O::enumerated()
    }

    fn accepts_arguments() -> bool {
        true
    }

    fn into_command_handler(self) -> Arc<dyn CommandHandler> {
        Arc::new(DialectHandler::<WithResourcesAndArgs<P, A, O>, F> {
            handler: self,
            _marker: PhantomData,
        })
    }
}

#[async_trait]
impl<F, P, A, Fut, O> CommandHandler for DialectHandler<WithResourcesAndArgs<P, A, O>, F>
where
    F: Fn(P, CommandContext, A) -> Fut + Send + Sync + 'static,
    P: ResourceParams,
    A: FromCommandArgs + Send + Sync + 'static,
    Fut: Future<Output = Result<O>> + Send,
    O: ResourceOutput,
{
    async fn call(&self, context: CommandContext) -> Result<CommandOutput> {
        let params = P::extract(&context)?;
        let args = A::from_command_args(&context)?;
        let output = (self.handler)(params, context, args).await?;
        Ok(output.into_command_output())
    }
}

impl<F, Fut, O> ResourceDialect<ContextOnly<O>> for F
where
    F: Fn(CommandContext) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<O>> + Send,
    O: ResourceOutput,
{
    fn resource_uses() -> Vec<ResourceUse> {
        Vec::new()
    }

    fn granted() -> Vec<&'static str> {
        O::granted()
    }

    fn enumerated() -> Vec<&'static str> {
        O::enumerated()
    }

    fn accepts_arguments() -> bool {
        false
    }

    fn into_command_handler(self) -> Arc<dyn CommandHandler> {
        Arc::new(DialectHandler::<ContextOnly<O>, F> {
            handler: self,
            _marker: PhantomData,
        })
    }
}

#[async_trait]
impl<F, Fut, O> CommandHandler for DialectHandler<ContextOnly<O>, F>
where
    F: Fn(CommandContext) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<O>> + Send,
    O: ResourceOutput,
{
    async fn call(&self, context: CommandContext) -> Result<CommandOutput> {
        let output = (self.handler)(context).await?;
        Ok(output.into_command_output())
    }
}

impl<F, A, Fut, O> ResourceDialect<ContextAndArgs<A, O>> for F
where
    F: Fn(CommandContext, A) -> Fut + Send + Sync + 'static,
    A: FromCommandArgs + Send + Sync + 'static,
    Fut: Future<Output = Result<O>> + Send,
    O: ResourceOutput,
{
    fn resource_uses() -> Vec<ResourceUse> {
        Vec::new()
    }

    fn granted() -> Vec<&'static str> {
        O::granted()
    }

    fn enumerated() -> Vec<&'static str> {
        O::enumerated()
    }

    fn accepts_arguments() -> bool {
        true
    }

    fn into_command_handler(self) -> Arc<dyn CommandHandler> {
        Arc::new(DialectHandler::<ContextAndArgs<A, O>, F> {
            handler: self,
            _marker: PhantomData,
        })
    }
}

#[async_trait]
impl<F, A, Fut, O> CommandHandler for DialectHandler<ContextAndArgs<A, O>, F>
where
    F: Fn(CommandContext, A) -> Fut + Send + Sync + 'static,
    A: FromCommandArgs + Send + Sync + 'static,
    Fut: Future<Output = Result<O>> + Send,
    O: ResourceOutput,
{
    async fn call(&self, context: CommandContext) -> Result<CommandOutput> {
        let args = A::from_command_args(&context)?;
        let output = (self.handler)(context, args).await?;
        Ok(output.into_command_output())
    }
}
