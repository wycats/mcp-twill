use std::{
    collections::{BTreeMap, VecDeque},
    convert::Infallible,
    future::Future,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

use bytes::Bytes;
use http::{HeaderMap, Method, Request, Response, StatusCode, header::CONTENT_TYPE};
use http_body::{Body, Frame, SizeHint};
use serde_json::{Map, Value, json};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::sync::{Mutex, mpsc};
use tower_service::Service;

use crate::{CliMcpServer, FrameworkError};
use rmcp::model::Extensions;

const PROTOCOL_VERSION: &str = "2026-07-28";
const EXPECTED_FINAL_RELEASE_COMMIT: Option<&str> = None;

pub struct StatelessMcpService {
    server: CliMcpServer,
}

#[derive(Clone)]
pub struct StatelessMcpHttpService {
    server: CliMcpServer,
}

pub struct StatelessMcpHttpBody {
    frames: VecDeque<Bytes>,
    streaming: Option<mpsc::Receiver<Bytes>>,
    abort: Option<tokio::task::AbortHandle>,
}

impl CliMcpServer {
    pub fn into_stateless_service(self) -> crate::Result<StatelessMcpService> {
        self.into_stateless_service_with_evidence(release_evidence_is_sealed())
    }

    pub(crate) fn into_stateless_service_with_evidence(
        self,
        sealed: bool,
    ) -> crate::Result<StatelessMcpService> {
        if !self.stateless_compatible() {
            return Err(FrameworkError::Build(
                "stateless MCP serving requires a native `2026-07-28` surface".to_string(),
            ));
        }
        if !sealed {
            return Err(FrameworkError::ProtocolReleaseUnsealed);
        }
        Ok(StatelessMcpService { server: self })
    }
}

impl StatelessMcpService {
    pub async fn serve_stdio<R, W>(self, reader: R, writer: W) -> crate::Result<()>
    where
        R: AsyncRead + Unpin + Send + 'static,
        W: AsyncWrite + Unpin + Send + 'static,
    {
        let mut lines = BufReader::new(reader).lines();
        let writer = Arc::new(Mutex::new(writer));
        let in_flight = Arc::new(Mutex::new(
            BTreeMap::<String, tokio::task::AbortHandle>::new(),
        ));
        while let Some(line) = lines
            .next_line()
            .await
            .map_err(|error| FrameworkError::Handler(error.to_string()))?
        {
            let bytes = Bytes::from(line);
            if is_cancellation_notification(&bytes) {
                if let Some(request_id) = cancellation_request_id(&bytes)
                    && let Some(handle) = in_flight
                        .lock()
                        .await
                        .remove(&canonical_request_id(&request_id))
                {
                    handle.abort();
                }
                continue;
            }
            let parsed = crate::stateless_wire::parse(
                &bytes,
                self.server.stateless_tasks_extension_enabled(),
            );
            if let Ok(request) = &parsed
                && !request.has_id
            {
                continue;
            }

            let request_id = parsed
                .as_ref()
                .ok()
                .filter(|request| request.has_id)
                .map(|request| canonical_request_id(&request.id));
            if let Some(request_id) = &request_id
                && in_flight.lock().await.contains_key(request_id)
            {
                write_stdio_response(
                    &writer,
                    response(
                        StatusCode::OK,
                        parsed
                            .as_ref()
                            .map(|request| request.id.clone())
                            .unwrap_or(Value::Null),
                        -32600,
                        "Invalid Request",
                        None,
                    )
                    .body,
                )
                .await?;
                continue;
            }

            let server = self.server.clone();
            let writer = writer.clone();
            let in_flight_for_task = in_flight.clone();
            let request_id_for_task = request_id.clone();
            let (start, ready) = tokio::sync::oneshot::channel();
            let task = tokio::spawn(async move {
                let _ = ready.await;
                let dispatched = dispatch_bytes(&server, None, bytes, &Extensions::new()).await;
                if let Some(request_id) = &request_id_for_task {
                    in_flight_for_task.lock().await.remove(request_id);
                }
                let _ = write_stdio_response(&writer, dispatched.body).await;
            });
            if let Some(request_id) = request_id {
                in_flight
                    .lock()
                    .await
                    .insert(request_id, task.abort_handle());
            }
            let _ = start.send(());
        }
        let handles = std::mem::take(&mut *in_flight.lock().await);
        for handle in handles.into_values() {
            handle.abort();
        }
        Ok(())
    }

    pub fn into_http_service(self) -> StatelessMcpHttpService {
        StatelessMcpHttpService {
            server: self.server,
        }
    }
}

fn is_cancellation_notification(bytes: &[u8]) -> bool {
    serde_json::from_slice::<Value>(bytes)
        .ok()
        .and_then(|value| {
            let object = value.as_object()?;
            Some(
                !object.contains_key("id")
                    && object.get("method").and_then(Value::as_str)
                        == Some("notifications/cancelled"),
            )
        })
        .unwrap_or(false)
}

fn cancellation_request_id(bytes: &[u8]) -> Option<Value> {
    serde_json::from_slice::<Value>(bytes)
        .ok()?
        .pointer("/params/requestId")
        .cloned()
}

fn canonical_request_id(id: &Value) -> String {
    serde_json::to_string(id).unwrap_or_else(|_| "null".to_string())
}

async fn write_stdio_response<W>(writer: &Arc<Mutex<W>>, body: Bytes) -> crate::Result<()>
where
    W: AsyncWrite + Unpin + Send + 'static,
{
    let mut writer = writer.lock().await;
    writer
        .write_all(&body)
        .await
        .map_err(|error| FrameworkError::Handler(error.to_string()))?;
    writer
        .write_all(b"\n")
        .await
        .map_err(|error| FrameworkError::Handler(error.to_string()))?;
    writer
        .flush()
        .await
        .map_err(|error| FrameworkError::Handler(error.to_string()))
}

impl Service<Request<Bytes>> for StatelessMcpHttpService {
    type Response = Response<StatelessMcpHttpBody>;
    type Error = Infallible;
    type Future = Pin<
        Box<dyn Future<Output = std::result::Result<Self::Response, Self::Error>> + Send + 'static>,
    >;

    fn poll_ready(
        &mut self,
        _context: &mut Context<'_>,
    ) -> Poll<std::result::Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, request: Request<Bytes>) -> Self::Future {
        let server = self.server.clone();
        Box::pin(async move {
            let (parts, body) = request.into_parts();
            let extensions = parts
                .extensions
                .get::<std::sync::Arc<Extensions>>()
                .cloned()
                .unwrap_or_else(|| std::sync::Arc::new(Extensions::new()));
            match preflight(&server, Some(&parts.method), Some(&parts.headers), &body) {
                Ok(request) if request.method == "tools/call" => {
                    let (sender, receiver) = mpsc::channel(8);
                    let (progress_sender, mut progress_receiver) = mpsc::channel(8);
                    let mut dispatched = Box::pin(async move {
                        dispatch_request(&server, request, &extensions, Some(progress_sender)).await
                    });
                    tokio::select! {
                        biased;
                        first = progress_receiver.recv() => {
                            let Some(first) = first else {
                                let dispatched = dispatched.await;
                                let mut response = Response::new(
                                    StatelessMcpHttpBody::immediate(dispatched.body),
                                );
                                *response.status_mut() = dispatched.status;
                                response.headers_mut().insert(
                                    CONTENT_TYPE,
                                    http::HeaderValue::from_static("application/json"),
                                );
                                return Ok(response);
                            };
                            let task = tokio::spawn(async move {
                                if sender.send(sse_progress(first)).await.is_err() {
                                    return;
                                }
                                loop {
                                    tokio::select! {
                                        Some(progress) = progress_receiver.recv() => {
                                            if sender.send(sse_progress(progress)).await.is_err() {
                                                return;
                                            }
                                        }
                                        dispatched = &mut dispatched => {
                                            while let Ok(progress) = progress_receiver.try_recv() {
                                                if sender.send(sse_progress(progress)).await.is_err() {
                                                    return;
                                                }
                                            }
                                            let _ = sender.send(sse_message(dispatched.body)).await;
                                            return;
                                        }
                                    }
                                }
                            });
                            let mut response = Response::new(StatelessMcpHttpBody::streaming(
                                receiver,
                                task.abort_handle(),
                            ));
                            response.headers_mut().insert(
                                CONTENT_TYPE,
                                http::HeaderValue::from_static("text/event-stream"),
                            );
                            Ok(response)
                        }
                        dispatched = &mut dispatched => {
                            let mut progress = Vec::new();
                            while let Ok(message) = progress_receiver.try_recv() {
                                progress.push(message);
                            }
                            if !progress.is_empty() {
                                let mut frames = progress
                                    .into_iter()
                                    .map(sse_progress)
                                    .collect::<VecDeque<_>>();
                                frames.push_back(sse_message(dispatched.body));
                                let mut response = Response::new(
                                    StatelessMcpHttpBody::completed_stream(frames),
                                );
                                *response.status_mut() = dispatched.status;
                                response.headers_mut().insert(
                                    CONTENT_TYPE,
                                    http::HeaderValue::from_static("text/event-stream"),
                                );
                                return Ok(response);
                            }
                            let mut response = Response::new(
                                StatelessMcpHttpBody::immediate(dispatched.body),
                            );
                            *response.status_mut() = dispatched.status;
                            response.headers_mut().insert(
                                CONTENT_TYPE,
                                http::HeaderValue::from_static("application/json"),
                            );
                            Ok(response)
                        }
                    }
                }
                Ok(request) => {
                    let dispatched = dispatch_request(&server, request, &extensions, None).await;
                    let mut response =
                        Response::new(StatelessMcpHttpBody::immediate(dispatched.body));
                    *response.status_mut() = dispatched.status;
                    response.headers_mut().insert(
                        CONTENT_TYPE,
                        http::HeaderValue::from_static("application/json"),
                    );
                    Ok(response)
                }
                Err(dispatched) => {
                    let mut response =
                        Response::new(StatelessMcpHttpBody::immediate(dispatched.body));
                    *response.status_mut() = dispatched.status;
                    response.headers_mut().insert(
                        CONTENT_TYPE,
                        http::HeaderValue::from_static("application/json"),
                    );
                    Ok(response)
                }
            }
        })
    }
}

impl StatelessMcpHttpBody {
    fn immediate(bytes: Bytes) -> Self {
        Self {
            frames: VecDeque::from([bytes]),
            streaming: None,
            abort: None,
        }
    }

    fn streaming(receiver: mpsc::Receiver<Bytes>, abort: tokio::task::AbortHandle) -> Self {
        Self {
            frames: VecDeque::new(),
            streaming: Some(receiver),
            abort: Some(abort),
        }
    }

    fn completed_stream(frames: VecDeque<Bytes>) -> Self {
        Self {
            frames,
            streaming: None,
            abort: None,
        }
    }
}

impl Drop for StatelessMcpHttpBody {
    fn drop(&mut self) {
        if let Some(abort) = self.abort.take() {
            abort.abort();
        }
    }
}

impl Body for StatelessMcpHttpBody {
    type Data = Bytes;
    type Error = Infallible;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
    ) -> Poll<Option<std::result::Result<Frame<Self::Data>, Self::Error>>> {
        if let Some(bytes) = self.frames.pop_front() {
            return Poll::Ready(Some(Ok(Frame::data(bytes))));
        }
        let Some(receiver) = &mut self.streaming else {
            return Poll::Ready(None);
        };
        match Pin::new(receiver).poll_recv(context) {
            Poll::Ready(Some(bytes)) => Poll::Ready(Some(Ok(Frame::data(bytes)))),
            Poll::Ready(None) => {
                self.streaming = None;
                self.abort = None;
                Poll::Ready(None)
            }
            Poll::Pending => Poll::Pending,
        }
    }

    fn is_end_stream(&self) -> bool {
        self.frames.is_empty() && self.streaming.is_none()
    }

    fn size_hint(&self) -> SizeHint {
        if self.streaming.is_some() {
            SizeHint::default()
        } else {
            let bytes = self.frames.iter().map(Bytes::len).sum::<usize>() as u64;
            SizeHint::with_exact(bytes)
        }
    }
}

struct DispatchedResponse {
    status: StatusCode,
    body: Bytes,
}

async fn dispatch_bytes(
    server: &CliMcpServer,
    headers: Option<&HeaderMap>,
    body: Bytes,
    extensions: &Extensions,
) -> DispatchedResponse {
    let request = match preflight(server, None, headers, &body) {
        Ok(request) => request,
        Err(response) => return response,
    };
    dispatch_request(server, request, extensions, None).await
}

fn preflight(
    server: &CliMcpServer,
    http_method: Option<&Method>,
    headers: Option<&HeaderMap>,
    body: &[u8],
) -> std::result::Result<crate::stateless_wire::Request, DispatchedResponse> {
    if let (Some(http_method), Some(headers)) = (http_method, headers) {
        if http_method != Method::POST {
            return Err(response(
                StatusCode::METHOD_NOT_ALLOWED,
                Value::Null,
                -32600,
                "Invalid Request",
                None,
            ));
        }
        if headers.contains_key("origin") {
            return Err(response(
                StatusCode::FORBIDDEN,
                Value::Null,
                -32600,
                "Invalid Request",
                None,
            ));
        }
        if exact_header(headers, "content-type", "application/json").is_err()
            || !accepts_json_and_sse(headers)
        {
            return Err(response(
                StatusCode::BAD_REQUEST,
                Value::Null,
                -32001,
                "Header mismatch",
                None,
            ));
        }
    }
    let request = crate::stateless_wire::parse(body, server.stateless_tasks_extension_enabled())
        .map_err(|error| {
            let id = serde_json::from_slice::<Value>(body)
                .ok()
                .and_then(|value| value.get("id").cloned())
                .unwrap_or(Value::Null);
            match error {
                crate::stateless_wire::WireError::Parse => {
                    response(StatusCode::BAD_REQUEST, id, -32700, "Parse error", None)
                }
                crate::stateless_wire::WireError::InvalidRequest => {
                    response(StatusCode::BAD_REQUEST, id, -32600, "Invalid Request", None)
                }
                crate::stateless_wire::WireError::InvalidParams => {
                    response(StatusCode::BAD_REQUEST, id, -32602, "Invalid params", None)
                }
            }
        })?;
    let id = request.id.clone();
    let observed_version = request
        .params
        .get("_meta")
        .and_then(Value::as_object)
        .and_then(|meta| meta.get("io.modelcontextprotocol/protocolVersion"))
        .and_then(Value::as_str);

    if let Some(headers) = headers
        && validate_http_headers(headers, &request.method, &request.params, observed_version)
            .is_err()
    {
        return Err(response(
            StatusCode::BAD_REQUEST,
            id,
            -32001,
            "Header mismatch",
            None,
        ));
    }
    let Some(observed_version) = observed_version else {
        return Err(response(
            StatusCode::BAD_REQUEST,
            id,
            -32602,
            "Invalid params",
            None,
        ));
    };
    if observed_version != PROTOCOL_VERSION {
        return Err(response(
            StatusCode::BAD_REQUEST,
            id,
            -32004,
            "Unsupported protocol version",
            Some(json!({
                "supported": [PROTOCOL_VERSION],
                "requested": observed_version,
            })),
        ));
    }
    Ok(request)
}

fn accepts_json_and_sse(headers: &HeaderMap) -> bool {
    let mut json = false;
    let mut sse = false;
    for value in headers.get_all("accept") {
        let Ok(value) = value.to_str() else {
            return false;
        };
        for value in value.split(',').map(str::trim) {
            json |= value == "application/json";
            sse |= value == "text/event-stream";
        }
    }
    json && sse
}

async fn dispatch_request(
    server: &CliMcpServer,
    request: crate::stateless_wire::Request,
    extensions: &Extensions,
    progress: Option<mpsc::Sender<Value>>,
) -> DispatchedResponse {
    let id = request.id;
    let method = request.method;
    let params = request.params;

    match server
        .dispatch_stateless(&method, params, extensions, progress)
        .await
    {
        Ok(mut result) => {
            if let Some(result) = result.as_object_mut() {
                result
                    .entry("resultType".to_string())
                    .or_insert_with(|| Value::String("complete".to_string()));
            }
            success(id, result)
        }
        Err(error) => {
            let status = if error.code == -32601 {
                StatusCode::NOT_FOUND
            } else {
                StatusCode::OK
            };
            response(status, id, error.code, error.message, error.data)
        }
    }
}

fn sse_message(body: Bytes) -> Bytes {
    let mut frame = Vec::with_capacity(body.len() + 23);
    frame.extend_from_slice(b"event: message\ndata: ");
    frame.extend_from_slice(&body);
    frame.extend_from_slice(b"\n\n");
    Bytes::from(frame)
}

fn sse_progress(progress: Value) -> Bytes {
    sse_message(Bytes::from(
        serde_json::to_vec(&progress).expect("progress notification serializes"),
    ))
}

fn validate_http_headers(
    headers: &HeaderMap,
    method: &str,
    params: &Map<String, Value>,
    observed_version: Option<&str>,
) -> std::result::Result<(), ()> {
    exact_header(headers, "mcp-protocol-version", observed_version.ok_or(())?)?;
    exact_header(headers, "mcp-method", method)?;
    let routed_name = match method {
        "tools/call" | "prompts/get" => params.get("name").and_then(Value::as_str),
        "resources/read" => params.get("uri").and_then(Value::as_str),
        "tasks/get" | "tasks/update" | "tasks/cancel" => {
            params.get("taskId").and_then(Value::as_str)
        }
        _ => None,
    };
    if let Some(routed_name) = routed_name {
        exact_header(headers, "mcp-name", routed_name)?;
    }
    Ok(())
}

fn exact_header(headers: &HeaderMap, name: &str, expected: &str) -> std::result::Result<(), ()> {
    let mut values = headers.get_all(name).iter();
    let value = values.next().ok_or(())?;
    if values.next().is_some() || value.to_str().map_err(|_| ())? != expected {
        return Err(());
    }
    Ok(())
}

fn success(id: Value, result: Value) -> DispatchedResponse {
    let body = serde_json::to_vec(&json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": result,
    }))
    .expect("JSON-RPC success response serializes");
    DispatchedResponse {
        status: StatusCode::OK,
        body: Bytes::from(body),
    }
}

fn response(
    status: StatusCode,
    id: Value,
    code: i32,
    message: impl Into<String>,
    data: Option<Value>,
) -> DispatchedResponse {
    let mut error = Map::from_iter([
        ("code".to_string(), Value::from(code)),
        ("message".to_string(), Value::String(message.into())),
    ]);
    if let Some(data) = data {
        error.insert("data".to_string(), data);
    }
    let body = serde_json::to_vec(&json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": error,
    }))
    .expect("JSON-RPC error response serializes");
    DispatchedResponse {
        status,
        body: Bytes::from(body),
    }
}

fn release_evidence_is_sealed() -> bool {
    let manifest: Value =
        serde_json::from_str(include_str!("../tests/fixtures/mcp/tasks/manifest.json"))
            .unwrap_or(Value::Null);
    let release = manifest.get("finalRelease").and_then(Value::as_object);
    match (EXPECTED_FINAL_RELEASE_COMMIT, release) {
        (Some(expected), Some(release)) => {
            release.get("tag").and_then(Value::as_str) == Some(PROTOCOL_VERSION)
                && release.get("peeledCommit").and_then(Value::as_str) == Some(expected)
        }
        _ => false,
    }
}

pub(crate) struct StatelessDispatchError {
    pub(crate) code: i32,
    pub(crate) message: &'static str,
    pub(crate) data: Option<Value>,
}

impl StatelessDispatchError {
    pub(crate) fn method_not_found() -> Self {
        Self {
            code: -32601,
            message: "Method not found",
            data: None,
        }
    }

    pub(crate) fn invalid_params(message: &'static str) -> Self {
        Self {
            code: -32602,
            message,
            data: None,
        }
    }

    pub(crate) fn internal(message: &'static str) -> Self {
        Self {
            code: -32603,
            message,
            data: None,
        }
    }

    pub(crate) fn missing_capability() -> Self {
        Self {
            code: -32003,
            message: "Missing required client capability",
            data: Some(json!({
                "requiredCapabilities": {
                    "extensions": { "io.modelcontextprotocol/tasks": {} }
                }
            })),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        future::poll_fn,
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        },
    };

    use http_body::Body;
    use serde_json::json;

    use super::*;
    use crate::{
        ApplicationResultContract, ApplicationSuccess, CliMcpServer, CommandRegistry, CommandSpec,
        DynamicCommandFailure, ExtensionOptionalPolicy, FrameworkHelpProjection, InMemoryTaskStore,
        McpProtocolTarget, NativeConfirmationRoute, NativeToolSurface, OutputContract,
        TaskAccessContext, TaskAccessPolicy, TaskAccessScope, TaskAccessScopeError,
        TaskAccessScopeProvider, TaskDeliveryDecl, TaskSupportSpec,
    };

    fn registry(support: TaskSupportSpec) -> CommandRegistry {
        let spec = CommandSpec::new(["work"], "Work", "Perform work")
            .task_support(support)
            .with_output(OutputContract {
                application: Some(ApplicationResultContract::new(json!({
                    "type": "object",
                    "properties": { "ok": { "type": "boolean" } },
                    "required": ["ok"],
                    "additionalProperties": false
                }))),
                ..OutputContract::default()
            });
        CommandRegistry::new("tasks", "Tasks").register_dynamic(spec, |_| async {
            Ok::<_, DynamicCommandFailure>(ApplicationSuccess::value(json!({ "ok": true })))
        })
    }

    fn service(support: TaskSupportSpec) -> StatelessMcpHttpService {
        service_from_registry(registry(support))
    }

    fn service_from_registry(registry: CommandRegistry) -> StatelessMcpHttpService {
        stateless_service_from_registry(registry).into_http_service()
    }

    fn stateless_service_from_registry(registry: CommandRegistry) -> StatelessMcpService {
        stateless_service_from_registry_with_access(registry, TaskAccessPolicy::CapabilityId)
    }

    fn stateless_service_from_registry_with_access(
        registry: CommandRegistry,
        access: TaskAccessPolicy,
    ) -> StatelessMcpService {
        let surface = NativeToolSurface::builder("tasks")
            .framework_help(FrameworkHelpProjection::Omitted)
            .confirmation_route(NativeConfirmationRoute::Unavailable)
            .task_delivery(TaskDeliveryDecl::tasks_extension(
                ExtensionOptionalPolicy::DeferredWhenAvailable,
                60_000,
            ))
            .direct("work", "work")
            .build(&registry, McpProtocolTarget::V2026_07_28)
            .unwrap();
        CliMcpServer::builder(registry)
            .surface(surface)
            .task_runtime(InMemoryTaskStore::server_instance(), access)
            .build()
            .unwrap()
            .into_stateless_service_with_evidence(true)
            .unwrap()
    }

    struct RefusingScope;

    impl TaskAccessScopeProvider for RefusingScope {
        fn scope(
            &self,
            _context: TaskAccessContext<'_>,
        ) -> std::result::Result<TaskAccessScope, TaskAccessScopeError> {
            Err(TaskAccessScopeError::new(std::io::Error::other(
                "private authentication failure",
            )))
        }
    }

    fn disabled_service() -> StatelessMcpHttpService {
        let registry = registry(TaskSupportSpec::Optional);
        let surface = NativeToolSurface::builder("tasks")
            .framework_help(FrameworkHelpProjection::Omitted)
            .confirmation_route(NativeConfirmationRoute::Unavailable)
            .direct("work", "work")
            .build(&registry, McpProtocolTarget::V2026_07_28)
            .unwrap();
        CliMcpServer::builder(registry)
            .surface(surface)
            .build()
            .unwrap()
            .into_stateless_service_with_evidence(true)
            .unwrap()
            .into_http_service()
    }

    fn meta(with_extension: bool) -> Value {
        json!({
            "io.modelcontextprotocol/clientCapabilities": {
                "extensions": if with_extension {
                    json!({ "io.modelcontextprotocol/tasks": {} })
                } else {
                    json!({})
                }
            },
            "io.modelcontextprotocol/clientInfo": {
                "name": "test",
                "version": "1"
            },
            "io.modelcontextprotocol/protocolVersion": PROTOCOL_VERSION
        })
    }

    fn request(id: u64, method: &str, name: Option<&str>, params: Value) -> Request<Bytes> {
        let mut builder = Request::builder()
            .method(Method::POST)
            .header(CONTENT_TYPE, "application/json")
            .header("Accept", "application/json, text/event-stream")
            .header("MCP-Protocol-Version", PROTOCOL_VERSION)
            .header("Mcp-Method", method);
        if let Some(name) = name {
            builder = builder.header("Mcp-Name", name);
        }
        builder
            .body(Bytes::from(
                serde_json::to_vec(&json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "method": method,
                    "params": params
                }))
                .unwrap(),
            ))
            .unwrap()
    }

    async fn response_value(response: Response<StatelessMcpHttpBody>) -> (StatusCode, Value) {
        let status = response.status();
        let mut body = Box::pin(response.into_body());
        let mut bytes = Vec::new();
        while let Some(frame) = poll_fn(|context| body.as_mut().poll_frame(context)).await {
            let frame = frame.unwrap();
            if let Ok(data) = frame.into_data() {
                bytes.extend_from_slice(&data);
            }
        }
        let bytes = bytes
            .strip_prefix(b"event: message\ndata: ")
            .and_then(|bytes| bytes.strip_suffix(b"\n\n"))
            .unwrap_or(&bytes);
        (status, serde_json::from_slice(bytes).unwrap())
    }

    #[tokio::test]
    async fn stateless_http_enforces_headers_versions_and_method_routing() {
        let mut service = service(TaskSupportSpec::Optional);
        let (status, value) = response_value(
            service
                .call(request(
                    1,
                    "tools/list",
                    None,
                    json!({ "_meta": meta(false) }),
                ))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert!(value["result"]["tools"].is_array());

        let (status, value) = response_value(
            service
                .call(request(
                    11,
                    "server/discover",
                    None,
                    json!({ "_meta": meta(false) }),
                ))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(value["result"]["resultType"], "complete");
        assert_eq!(
            value["result"]["supportedVersions"],
            json!([PROTOCOL_VERSION])
        );
        assert_eq!(
            value["result"]["capabilities"]["extensions"]["io.modelcontextprotocol/tasks"],
            json!({})
        );

        let bad = Request::builder()
            .method(Method::POST)
            .header(CONTENT_TYPE, "application/json")
            .header("Accept", "application/json, text/event-stream")
            .header("MCP-Protocol-Version", PROTOCOL_VERSION)
            .header("Mcp-Method", "wrong")
            .body(Bytes::from(
                serde_json::to_vec(&json!({
                    "jsonrpc": "2.0",
                    "id": 2,
                    "method": "tools/list",
                    "params": { "_meta": meta(false) }
                }))
                .unwrap(),
            ))
            .unwrap();
        let (status, value) = response_value(service.call(bad).await.unwrap()).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(value["error"]["code"], -32001);

        let with_origin = Request::builder()
            .method(Method::POST)
            .header(CONTENT_TYPE, "application/json")
            .header("Accept", "application/json, text/event-stream")
            .header("Origin", "https://untrusted.example")
            .header("MCP-Protocol-Version", PROTOCOL_VERSION)
            .header("Mcp-Method", "tools/list")
            .body(Bytes::from(
                serde_json::to_vec(&json!({
                    "jsonrpc": "2.0",
                    "id": 12,
                    "method": "tools/list",
                    "params": { "_meta": meta(false) }
                }))
                .unwrap(),
            ))
            .unwrap();
        let (status, _) = response_value(service.call(with_origin).await.unwrap()).await;
        assert_eq!(status, StatusCode::FORBIDDEN);

        let (status, value) = response_value(
            service
                .call(request(
                    3,
                    "unknown/method",
                    None,
                    json!({ "_meta": meta(false) }),
                ))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(value["error"]["code"], -32601);
    }

    #[tokio::test]
    async fn extension_materializes_and_polls_a_task() {
        let mut service = service(TaskSupportSpec::Required);
        let (_, created) = response_value(
            service
                .call(request(
                    1,
                    "tools/call",
                    Some("work"),
                    json!({
                        "_meta": meta(true),
                        "name": "work",
                        "arguments": {}
                    }),
                ))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(created["result"]["resultType"], "task");
        assert_eq!(created["result"]["status"], "working");
        let task_id = created["result"]["taskId"].as_str().unwrap().to_string();
        assert_eq!(task_id.len(), 64);

        let mut observed = Value::Null;
        for id in 2..22 {
            let (_, value) = response_value(
                service
                    .call(request(
                        id,
                        "tasks/get",
                        Some(&task_id),
                        json!({ "_meta": meta(true), "taskId": task_id.clone() }),
                    ))
                    .await
                    .unwrap(),
            )
            .await;
            observed = value;
            if observed["result"]["status"] == "completed" {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert_eq!(observed["result"]["status"], "completed");
        assert_eq!(observed["result"]["result"]["isError"], false);
        assert_eq!(observed["result"]["result"]["resultType"], "complete");
        assert!(
            observed["result"]["createdAt"]
                .as_str()
                .unwrap()
                .ends_with('Z')
        );
    }

    #[tokio::test]
    async fn required_extension_capability_fails_before_task_creation() {
        let mut service = service(TaskSupportSpec::Required);
        let (_, value) = response_value(
            service
                .call(request(
                    1,
                    "tools/call",
                    Some("work"),
                    json!({
                        "_meta": meta(false),
                        "name": "work",
                        "arguments": {}
                    }),
                ))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(value["error"]["code"], -32003);
    }

    #[tokio::test]
    async fn creation_scope_failure_uses_the_static_access_error() {
        let registry = registry(TaskSupportSpec::Optional);
        let mut service = stateless_service_from_registry_with_access(
            registry,
            TaskAccessPolicy::Scoped(Arc::new(RefusingScope)),
        )
        .into_http_service();
        let (_, value) = response_value(
            service
                .call(request(
                    1,
                    "tools/call",
                    Some("work"),
                    json!({
                        "_meta": meta(true),
                        "name": "work",
                        "arguments": {}
                    }),
                ))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(value["error"]["code"], -32603);
        assert_eq!(value["error"]["message"], "Task access scope unavailable");
        assert!(!value.to_string().contains("private authentication failure"));
    }

    #[tokio::test]
    async fn disabled_delivery_rejects_task_methods_before_extension_decoding() {
        let mut service = disabled_service();
        let (_, value) = response_value(
            service
                .call(request(
                    1,
                    "tasks/get",
                    Some("not-a-task-id"),
                    json!({
                        "_meta": meta(false),
                        "taskId": "not-a-task-id",
                        "inputResponses": 1
                    }),
                ))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(value["error"]["code"], -32601);
    }

    #[tokio::test]
    async fn extension_cancellation_is_cooperative_and_idempotent() {
        let spec = CommandSpec::new(["work"], "Work", "Perform work")
            .task_support(TaskSupportSpec::Required)
            .with_output(OutputContract {
                application: Some(ApplicationResultContract::new(json!({
                    "type": "object",
                    "properties": { "ok": { "type": "boolean" } },
                    "required": ["ok"],
                    "additionalProperties": false
                }))),
                ..OutputContract::default()
            });
        let registry = CommandRegistry::new("tasks", "Tasks").register_dynamic(spec, |_| async {
            tokio::time::sleep(std::time::Duration::from_secs(60)).await;
            Ok::<_, DynamicCommandFailure>(ApplicationSuccess::value(json!({ "ok": true })))
        });
        let mut service = service_from_registry(registry);
        let (_, created) = response_value(
            service
                .call(request(
                    1,
                    "tools/call",
                    Some("work"),
                    json!({
                        "_meta": meta(true),
                        "name": "work",
                        "arguments": {}
                    }),
                ))
                .await
                .unwrap(),
        )
        .await;
        let task_id = created["result"]["taskId"].as_str().unwrap().to_string();

        for id in [2, 3] {
            let (_, acknowledgement) = response_value(
                service
                    .call(request(
                        id,
                        "tasks/cancel",
                        Some(&task_id),
                        json!({ "_meta": meta(true), "taskId": task_id.clone() }),
                    ))
                    .await
                    .unwrap(),
            )
            .await;
            assert_eq!(
                acknowledgement["result"],
                json!({ "resultType": "complete" })
            );
        }

        let mut observed = Value::Null;
        for id in 4..24 {
            let (_, value) = response_value(
                service
                    .call(request(
                        id,
                        "tasks/get",
                        Some(&task_id),
                        json!({ "_meta": meta(true), "taskId": task_id.clone() }),
                    ))
                    .await
                    .unwrap(),
            )
            .await;
            observed = value;
            if observed["result"]["status"] == "cancelled" {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        assert_eq!(observed["result"]["status"], "cancelled");
        assert_eq!(observed["result"]["error"]["error"]["code"], -32000);
    }

    #[tokio::test]
    async fn stdio_cancellation_suppresses_only_the_selected_live_request() {
        let spec = CommandSpec::new(["work"], "Work", "Perform work")
            .task_support(TaskSupportSpec::Optional)
            .with_output(OutputContract {
                application: Some(ApplicationResultContract::new(json!({
                    "type": "object",
                    "properties": { "ok": { "type": "boolean" } },
                    "required": ["ok"],
                    "additionalProperties": false
                }))),
                ..OutputContract::default()
            });
        let registry = CommandRegistry::new("tasks", "Tasks").register_dynamic(spec, |_| async {
            tokio::time::sleep(std::time::Duration::from_secs(60)).await;
            Ok::<_, DynamicCommandFailure>(ApplicationSuccess::value(json!({ "ok": true })))
        });
        let service = stateless_service_from_registry(registry);
        let (mut request_writer, request_reader) = tokio::io::duplex(16 * 1024);
        let (response_writer, response_reader) = tokio::io::duplex(16 * 1024);
        let serving = tokio::spawn(service.serve_stdio(request_reader, response_writer));
        let mut responses = BufReader::new(response_reader).lines();

        let call = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {
                "_meta": meta(false),
                "name": "work",
                "arguments": {}
            }
        });
        request_writer
            .write_all(format!("{call}\n").as_bytes())
            .await
            .unwrap();
        let other = json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "unknown/method",
            "params": { "_meta": meta(false) }
        });
        request_writer
            .write_all(format!("{other}\n").as_bytes())
            .await
            .unwrap();
        let cancelled = json!({
            "jsonrpc": "2.0",
            "method": "notifications/cancelled",
            "params": { "requestId": 1, "reason": "private caller text" }
        });
        request_writer
            .write_all(format!("{cancelled}\n").as_bytes())
            .await
            .unwrap();

        let line = tokio::time::timeout(std::time::Duration::from_secs(1), responses.next_line())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        let response: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(response["id"], 2);
        assert_eq!(response["error"]["code"], -32601);
        assert!(!line.contains("private caller text"));

        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(25), responses.next_line())
                .await
                .is_err()
        );
        drop(request_writer);
        serving.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn malformed_stdio_cancellation_notification_is_silent() {
        let service = stateless_service_from_registry(registry(TaskSupportSpec::Optional));
        let (mut request_writer, request_reader) = tokio::io::duplex(4096);
        let (response_writer, response_reader) = tokio::io::duplex(4096);
        let serving = tokio::spawn(service.serve_stdio(request_reader, response_writer));
        let mut responses = BufReader::new(response_reader).lines();

        request_writer
            .write_all(
                b"{\"jsonrpc\":\"2.0\",\"method\":\"notifications/cancelled\",\"params\":false}\n",
            )
            .await
            .unwrap();
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(25), responses.next_line())
                .await
                .is_err()
        );

        drop(request_writer);
        serving.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn dropping_streaming_http_body_cancels_ordinary_request_work() {
        struct DropSignal(Arc<AtomicBool>);

        impl Drop for DropSignal {
            fn drop(&mut self) {
                self.0.store(true, Ordering::Release);
            }
        }

        let started = Arc::new(AtomicBool::new(false));
        let dropped = Arc::new(AtomicBool::new(false));
        let started_for_handler = started.clone();
        let dropped_for_handler = dropped.clone();
        let spec = CommandSpec::new(["work"], "Work", "Perform work")
            .task_support(TaskSupportSpec::Optional)
            .with_output(OutputContract {
                application: Some(ApplicationResultContract::new(json!({
                    "type": "object",
                    "properties": { "ok": { "type": "boolean" } },
                    "required": ["ok"],
                    "additionalProperties": false
                }))),
                ..OutputContract::default()
            });
        let registry = CommandRegistry::new("tasks", "Tasks").register_dynamic(spec, move |_| {
            let started = started_for_handler.clone();
            let dropped = dropped_for_handler.clone();
            async move {
                let _drop_signal = DropSignal(dropped);
                started.store(true, Ordering::Release);
                std::future::pending::<
                    std::result::Result<ApplicationSuccess<Value>, DynamicCommandFailure>,
                >()
                .await
            }
        });
        let mut service = service_from_registry(registry);
        let mut request_meta = meta(false);
        request_meta
            .as_object_mut()
            .unwrap()
            .insert("progressToken".to_string(), json!("drop-test"));
        let response = service
            .call(request(
                1,
                "tools/call",
                Some("work"),
                json!({
                    "_meta": request_meta,
                    "name": "work",
                    "arguments": {}
                }),
            ))
            .await
            .unwrap();
        for _ in 0..20 {
            if started.load(Ordering::Acquire) {
                break;
            }
            tokio::task::yield_now().await;
        }
        assert!(started.load(Ordering::Acquire));
        drop(response);
        for _ in 0..20 {
            if dropped.load(Ordering::Acquire) {
                break;
            }
            tokio::task::yield_now().await;
        }
        assert!(dropped.load(Ordering::Acquire));
    }

    #[tokio::test]
    async fn immediate_http_tool_result_uses_one_json_frame() {
        let mut service = service(TaskSupportSpec::Optional);
        let response = service
            .call(request(
                1,
                "tools/call",
                Some("work"),
                json!({
                    "_meta": meta(false),
                    "name": "work",
                    "arguments": {}
                }),
            ))
            .await
            .unwrap();
        assert_eq!(
            response.headers().get(CONTENT_TYPE).unwrap(),
            "application/json"
        );
        let mut body = Box::pin(response.into_body());
        let first = poll_fn(|context| body.as_mut().poll_frame(context))
            .await
            .unwrap()
            .unwrap()
            .into_data()
            .unwrap();
        let value: Value = serde_json::from_slice(&first).unwrap();
        assert_eq!(value["result"]["resultType"], "complete");
        assert!(
            poll_fn(|context| body.as_mut().poll_frame(context))
                .await
                .is_none()
        );
    }

    #[tokio::test]
    async fn streaming_http_emits_request_progress_before_the_final_response() {
        let mut service = service(TaskSupportSpec::Optional);
        let mut request_meta = meta(false);
        request_meta
            .as_object_mut()
            .unwrap()
            .insert("progressToken".to_string(), json!("request-progress"));
        let response = service
            .call(request(
                1,
                "tools/call",
                Some("work"),
                json!({
                    "_meta": request_meta,
                    "name": "work",
                    "arguments": {}
                }),
            ))
            .await
            .unwrap();
        assert_eq!(
            response.headers().get(CONTENT_TYPE).unwrap(),
            "text/event-stream"
        );
        let mut body = Box::pin(response.into_body());
        let mut messages = Vec::new();
        while let Some(frame) = poll_fn(|context| body.as_mut().poll_frame(context)).await {
            let data = frame.unwrap().into_data().unwrap();
            let data = data
                .strip_prefix(b"event: message\ndata: ")
                .and_then(|data| data.strip_suffix(b"\n\n"))
                .unwrap();
            messages.push(serde_json::from_slice::<Value>(data).unwrap());
        }
        assert_eq!(messages.len(), 5, "{messages:#?}");
        for (expected, message) in [1_u64, 2, 4, 5].into_iter().zip(&messages[..4]) {
            assert_eq!(message["method"], "notifications/progress");
            assert_eq!(message["params"]["progressToken"], "request-progress");
            assert_eq!(
                message["params"]["progress"].as_f64(),
                Some(expected as f64)
            );
        }
        assert_eq!(messages[4]["id"], 1);
        assert_eq!(messages[4]["result"]["resultType"], "complete");
    }
}
