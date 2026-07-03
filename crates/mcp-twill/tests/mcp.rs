use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use futures::StreamExt;
use mcp_twill::{
    ArgSpec, CliMcpServer, CliMcpServerConfig, CommandOutput, CommandRegistry, CommandSpec,
    OutputFormat, OutputSpec, PermissionEffect, PermissionSpec, ResponseProfile, RunRequest,
};
use rmcp::{
    ClientHandler, ServiceExt,
    handler::client::progress::ProgressDispatcher,
    model::{
        CallToolRequestParams, ClientRequest, GetPromptRequestParams, GetTaskInfoParams,
        GetTaskResultParams, ProgressNotificationParam, ReadResourceRequestParams, Request,
        ServerResult, TaskStatus,
    },
    service::PeerRequestOptions,
};
use serde_json::{Value, json};

fn json_object<T: serde::Serialize>(value: T) -> anyhow::Result<serde_json::Map<String, Value>> {
    match serde_json::to_value(value)? {
        Value::Object(map) => Ok(map),
        other => anyhow::bail!("expected JSON object, got {other:?}"),
    }
}

fn request(command: &str, args: serde_json::Value) -> anyhow::Result<RunRequest> {
    Ok(RunRequest {
        command: command.to_string(),
        args: serde_json::from_value(args)?,
        stdin: None,
        output: None,
        mode: mcp_twill::RunMode::Execute,
        approval: None,
        dry_run: false,
    })
}

fn write_request(title: &str) -> anyhow::Result<RunRequest> {
    request(
        "issues create --title $args.title",
        json!({ "title": title }),
    )
}

fn counted_write_registry(dispatches: Arc<AtomicUsize>) -> CommandRegistry {
    registry().register(
        CommandSpec::new(["issues", "create"], "Create issue", "Create issue")
            .with_arg(ArgSpec::string("title", "Issue title"))
            .with_permission(PermissionSpec::new(
                PermissionEffect::Write,
                "issues",
                "Creates issues",
            )),
        move |_context| {
            let dispatches = dispatches.clone();
            async move {
                dispatches.fetch_add(1, Ordering::SeqCst);
                Ok(CommandOutput::structured(
                    json!({ "id": 3, "title": "Created" }),
                ))
            }
        },
    )
}

fn custom_effect_registry(dispatches: Arc<AtomicUsize>) -> CommandRegistry {
    registry().register(
        CommandSpec::new(["issues", "sync"], "Sync issues", "Sync issues").with_permission(
            PermissionSpec::new(
                PermissionEffect::Custom("sync".to_string()),
                "issues",
                "Syncs issues through a custom effect",
            ),
        ),
        move |_context| {
            let dispatches = dispatches.clone();
            async move {
                dispatches.fetch_add(1, Ordering::SeqCst);
                Ok(CommandOutput::structured(json!({ "synced": true })))
            }
        },
    )
}

fn registry() -> CommandRegistry {
    CommandRegistry::new("mcp-test", "MCP integration test server").register(
        CommandSpec::new(["issues", "list"], "List issues", "List issues").with_permission(
            PermissionSpec::new(PermissionEffect::Read, "issues", "Reads issues"),
        ),
        |_context| async {
            Ok(CommandOutput::structured(json!([
                { "id": 1, "title": "One", "body": "Body 1" },
                { "id": 2, "title": "Two", "body": "Body 2" }
            ])))
        },
    )
}

fn write_registry() -> CommandRegistry {
    registry().register(
        CommandSpec::new(["issues", "create"], "Create issue", "Create issue")
            .with_arg(ArgSpec::string("title", "Issue title"))
            .with_permission(PermissionSpec::new(
                PermissionEffect::Write,
                "issues",
                "Creates issues",
            )),
        |_context| async {
            Ok(CommandOutput::structured(
                json!({ "id": 3, "title": "Created" }),
            ))
        },
    )
}

struct TestClient {
    progress: ProgressDispatcher,
}

impl TestClient {
    fn new() -> Self {
        Self {
            progress: ProgressDispatcher::new(),
        }
    }
}

impl ClientHandler for TestClient {
    async fn on_progress(
        &self,
        params: ProgressNotificationParam,
        _context: rmcp::service::NotificationContext<rmcp::RoleClient>,
    ) {
        self.progress.handle_notification(params).await;
    }
}

/// A client that declares the MCP roots capability and serves `roots/list`.
struct RootsClient {
    roots: Vec<rmcp::model::Root>,
}

impl ClientHandler for RootsClient {
    fn get_info(&self) -> rmcp::model::ClientInfo {
        let mut info = rmcp::model::ClientInfo::default();
        info.capabilities = rmcp::model::ClientCapabilities::builder()
            .enable_roots()
            .build();
        info
    }

    async fn list_roots(
        &self,
        _context: rmcp::service::RequestContext<rmcp::RoleClient>,
    ) -> Result<rmcp::model::ListRootsResult, rmcp::ErrorData> {
        Ok(rmcp::model::ListRootsResult::new(self.roots.clone()))
    }
}

#[tokio::test]
async fn getting_started_prompt_includes_declared_guidance() -> anyhow::Result<()> {
    let (server_transport, client_transport) = tokio::io::duplex(8192);
    let server = CliMcpServer::new(registry().declare_guidance(
        mcp_twill::CommandGuidance::run_command(
            "quickstart",
            "getting-started",
            "issues list",
        ),
    ))?;
    tokio::spawn(async move {
        server.serve(server_transport).await?.waiting().await?;
        anyhow::Ok(())
    });

    let client = TestClient::new().serve(client_transport).await?;
    let prompt = client
        .get_prompt(GetPromptRequestParams::new("getting_started"))
        .await?;
    let text = serde_json::to_string(&prompt)?;
    assert!(text.contains("Guidance:"), "{text}");
    assert!(text.contains("issues list"), "{text}");

    client.cancel().await?;
    Ok(())
}

#[tokio::test]
async fn mcp_exposes_two_tools_and_resources_prompts() -> anyhow::Result<()> {
    let (server_transport, client_transport) = tokio::io::duplex(8192);
    let server = CliMcpServer::new(registry())?;
    let server_handle = tokio::spawn(async move {
        server.serve(server_transport).await?.waiting().await?;
        anyhow::Ok(())
    });

    let client = TestClient::new().serve(client_transport).await?;

    let tools = client.list_tools(Default::default()).await?;
    let names: Vec<_> = tools.tools.iter().map(|tool| tool.name.as_ref()).collect();
    assert_eq!(names, vec!["help", "run"]);
    let run = tools.tools.iter().find(|tool| tool.name == "run").unwrap();
    assert_eq!(run.task_support(), rmcp::model::TaskSupport::Optional);
    assert_eq!(run.annotations.as_ref().unwrap().read_only_hint, Some(true));

    let resources = client.list_resources(Default::default()).await?;
    assert!(
        resources
            .resources
            .iter()
            .any(|r| r.uri == "cli://commands")
    );
    assert!(resources.resources.iter().any(|r| r.uri == "cli://catalog"));
    assert!(resources.resources.iter().any(|r| r.uri == "cli://lanes"));
    let resource = client
        .read_resource(ReadResourceRequestParams::new("cli://commands"))
        .await?;
    assert!(matches!(
        resource.contents.first().unwrap(),
        rmcp::model::ResourceContents::TextResourceContents { text, .. } if text.contains("issues list")
    ));

    let prompts = client.list_prompts(Default::default()).await?;
    assert_eq!(prompts.prompts[0].name, "getting_started");
    let prompt = client
        .get_prompt(GetPromptRequestParams::new("getting_started"))
        .await?;
    assert_eq!(prompt.messages.len(), 1);
    assert!(serde_json::to_string(&prompt)?.contains("Start execution"));

    client.cancel().await?;
    server_handle.await??;
    Ok(())
}

#[test]
fn server_runtime_identity_includes_the_crate_version() -> anyhow::Result<()> {
    let server = CliMcpServer::new(registry())?;
    let identity = server.runtime_identity();

    assert_eq!(identity.server_name, server.registry().server_name());
    assert_eq!(identity.server_version.as_deref(), Some(env!("CARGO_PKG_VERSION")));
    assert_eq!(
        identity.catalog_hash,
        server.registry().catalog_identity().catalog_hash
    );
    Ok(())
}

#[tokio::test]
async fn mcp_run_emits_progress_and_returns_structured_content() -> anyhow::Result<()> {
    let (server_transport, client_transport) = tokio::io::duplex(8192);
    let server = CliMcpServer::new(registry())?;
    tokio::spawn(async move {
        server.serve(server_transport).await?.waiting().await?;
        anyhow::Ok(())
    });

    let client_handler = TestClient::new();
    let dispatcher = client_handler.progress.clone();
    let client = client_handler.serve(client_transport).await?;

    let request = RunRequest {
        command: "issues list".to_string(),
        args: BTreeMap::new(),
        stdin: None,
        output: None,
        mode: mcp_twill::RunMode::Execute,
        approval: None,
        dry_run: false,
    };
    let params = CallToolRequestParams::new("run").with_arguments(json_object(request)?);
    let handle = client
        .send_cancellable_request(
            ClientRequest::CallToolRequest(Request::new(params)),
            PeerRequestOptions::no_options(),
        )
        .await?;
    let mut progress = dispatcher.subscribe(handle.progress_token.clone()).await;
    let result = handle.await_response().await?;
    let mut seen = Vec::new();
    while let Ok(Some(notification)) =
        tokio::time::timeout(std::time::Duration::from_millis(50), progress.next()).await
    {
        seen.push(notification.message.unwrap_or_default());
        if seen.len() >= 4 {
            break;
        }
    }

    let value = serde_json::to_value(result)?;
    let structured = &value["structuredContent"];
    assert_eq!(structured["status"], "ok");
    assert_eq!(structured["command"], json!(["issues", "list"]));
    assert!(
        structured["output"]["structured"]
            .to_string()
            .contains("One")
    );
    assert!(structured.get("plan").is_none());
    assert!(seen.iter().any(|message| message.contains("Parsing")));
    assert!(seen.iter().any(|message| message.contains("complete")));

    client.cancel().await?;
    Ok(())
}

#[tokio::test]
async fn task_augmented_run_completes_when_negotiated() -> anyhow::Result<()> {
    let (server_transport, client_transport) = tokio::io::duplex(8192);
    let server = CliMcpServer::new(registry())?;
    tokio::spawn(async move {
        server.serve(server_transport).await?.waiting().await?;
        anyhow::Ok(())
    });

    let client = TestClient::new().serve(client_transport).await?;
    let request = RunRequest {
        command: "issues list".to_string(),
        args: BTreeMap::new(),
        stdin: None,
        output: None,
        mode: mcp_twill::RunMode::Execute,
        approval: None,
        dry_run: false,
    };
    let params = CallToolRequestParams::new("run")
        .with_arguments(json_object(request)?)
        .with_task(serde_json::Map::new());
    let created = client
        .send_request(ClientRequest::CallToolRequest(Request::new(params)))
        .await?;
    let ServerResult::CreateTaskResult(created) = created else {
        panic!("expected CreateTaskResult, got {created:?}");
    };
    let task_id = created.task.task_id.clone();

    let mut task = match client
        .send_request(ClientRequest::GetTaskInfoRequest(Request::new(
            GetTaskInfoParams {
                meta: None,
                task_id: task_id.clone(),
            },
        )))
        .await?
    {
        ServerResult::GetTaskResult(task) => task,
        other => panic!("expected GetTaskResult, got {other:?}"),
    };
    for _ in 0..20 {
        if task.task.status == TaskStatus::Completed {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        task = match client
            .send_request(ClientRequest::GetTaskInfoRequest(Request::new(
                GetTaskInfoParams {
                    meta: None,
                    task_id: task_id.clone(),
                },
            )))
            .await?
        {
            ServerResult::GetTaskResult(task) => task,
            other => panic!("expected GetTaskResult, got {other:?}"),
        };
    }
    assert_eq!(task.task.status, TaskStatus::Completed);

    let payload = client
        .send_request(ClientRequest::GetTaskResultRequest(Request::new(
            GetTaskResultParams {
                meta: None,
                task_id,
            },
        )))
        .await?;
    let value: Value = match payload {
        ServerResult::GetTaskPayloadResult(payload) => serde_json::to_value(payload)?,
        ServerResult::CallToolResult(result) => serde_json::to_value(result)?,
        other => panic!("expected task payload result, got {other:?}"),
    };
    assert!(value.to_string().contains("issues"));

    client.cancel().await?;
    Ok(())
}

#[tokio::test]
async fn generated_effect_lane_tools_redirect_and_dispatch() -> anyhow::Result<()> {
    let (server_transport, client_transport) = tokio::io::duplex(8192);
    let server = CliMcpServer::with_config(
        write_registry(),
        CliMcpServerConfig::default().with_execution_tool_name("repo"),
    )?;
    tokio::spawn(async move {
        server.serve(server_transport).await?.waiting().await?;
        anyhow::Ok(())
    });

    let client = TestClient::new().serve(client_transport).await?;

    let tools = client.list_tools(Default::default()).await?;
    let names: Vec<_> = tools.tools.iter().map(|tool| tool.name.as_ref()).collect();
    assert_eq!(names, vec!["help", "repo", "repo-write"]);
    let repo = tools.tools.iter().find(|tool| tool.name == "repo").unwrap();
    let repo_write = tools
        .tools
        .iter()
        .find(|tool| tool.name == "repo-write")
        .unwrap();
    assert_eq!(
        repo.annotations.as_ref().unwrap().read_only_hint,
        Some(true)
    );
    assert_eq!(
        repo_write.annotations.as_ref().unwrap().read_only_hint,
        Some(false)
    );
    assert_eq!(
        repo_write.annotations.as_ref().unwrap().destructive_hint,
        Some(false)
    );
    assert!(repo.description.as_ref().unwrap().contains("Start here"));

    let request = RunRequest {
        command: "issues create --title $args.title".to_string(),
        args: serde_json::from_value(json!({ "title": "New" }))?,
        stdin: None,
        output: None,
        mode: mcp_twill::RunMode::Execute,
        approval: None,
        dry_run: false,
    };
    let wrong_lane = client
        .call_tool(CallToolRequestParams::new("repo").with_arguments(json_object(request.clone())?))
        .await?;
    assert_eq!(wrong_lane.is_error, Some(true));
    let structured = wrong_lane.structured_content.unwrap();
    assert_eq!(structured["status"], "wrongEffectLane");
    assert_eq!(structured["error"]["code"], "wrong_effect_lane");
    assert_eq!(structured["error"]["details"]["currentTool"], "repo");
    assert_eq!(structured["error"]["details"]["requiredTool"], "repo-write");
    assert_eq!(structured["retry"]["tool"], "repo-write");
    assert_eq!(
        structured["retry"]["arguments"]["command"],
        "issues create --title $args.title"
    );

    let permission_required = client
        .call_tool(
            CallToolRequestParams::new("repo-write").with_arguments(json_object(request.clone())?),
        )
        .await?;
    assert_eq!(permission_required.is_error, Some(true));
    let structured = permission_required.structured_content.unwrap();
    assert_eq!(structured["status"], "permissionRequired");
    assert_eq!(structured["error"]["code"], "permission_required");
    assert!(
        structured["preview"]["requiresConfirmation"]
            .as_bool()
            .unwrap()
    );
    assert_eq!(structured["steering"][0]["request"]["tool"], "repo-write");
    let token = structured["replay"]["token"].as_str().unwrap().to_string();
    let random_suffix = token.strip_prefix("replay-").unwrap();
    assert_eq!(random_suffix.len(), 64);
    assert!(random_suffix.chars().all(|value| value.is_ascii_hexdigit()));
    assert_ne!(token, "replay-1");
    assert!(
        !structured["display"]["summary"]
            .as_str()
            .unwrap()
            .contains(&token)
    );

    let mut approved = request;
    approved.approval = Some(mcp_twill::ApprovalInput {
        token,
        confirm: true,
    });
    let dispatched = client
        .call_tool(CallToolRequestParams::new("repo-write").with_arguments(json_object(approved)?))
        .await?;
    assert_eq!(dispatched.is_error, Some(false));
    let structured = dispatched.structured_content.unwrap();
    assert_eq!(structured["status"], "ok");
    assert_eq!(structured["output"]["structured"]["title"], "Created");

    client.cancel().await?;
    Ok(())
}

#[tokio::test]
async fn custom_effects_are_rejected_at_server_construction() -> anyhow::Result<()> {
    let dispatches = Arc::new(AtomicUsize::new(0));
    let result = CliMcpServer::with_config(
        custom_effect_registry(dispatches.clone()),
        CliMcpServerConfig::default().with_execution_tool_name("repo"),
    );
    let error = match result {
        Ok(_) => panic!("expected custom effect to fail server construction"),
        Err(error) => error,
    };
    let message = error.to_string();
    assert!(message.contains("issues sync"), "names the command: {message}");
    assert!(message.contains("sync"), "names the effect: {message}");
    assert!(
        message.contains("read, write, delete, exec, or network"),
        "names the standard effects: {message}"
    );
    assert_eq!(dispatches.load(Ordering::SeqCst), 0);
    Ok(())
}

#[tokio::test]
async fn preview_returns_permission_data_without_dispatch() -> anyhow::Result<()> {
    let dispatches = Arc::new(AtomicUsize::new(0));
    let (server_transport, client_transport) = tokio::io::duplex(8192);
    let server = CliMcpServer::with_config(
        counted_write_registry(dispatches.clone()),
        CliMcpServerConfig::default().with_execution_tool_name("repo"),
    )?;
    tokio::spawn(async move {
        server.serve(server_transport).await?.waiting().await?;
        anyhow::Ok(())
    });

    let client = TestClient::new().serve(client_transport).await?;
    let mut preview = write_request("Preview")?;
    preview.mode = mcp_twill::RunMode::Preview;
    let result = client
        .call_tool(CallToolRequestParams::new("repo-write").with_arguments(json_object(preview)?))
        .await?;
    assert_eq!(result.is_error, Some(false));
    let structured = result.structured_content.unwrap();
    assert_eq!(structured["status"], "ok");
    assert_eq!(structured["preview"]["lane"], "write");
    assert_eq!(structured["preview"]["requiresConfirmation"], true);
    assert_eq!(dispatches.load(Ordering::SeqCst), 0);

    client.cancel().await?;
    Ok(())
}

#[tokio::test]
async fn dry_run_mode_returns_plan_without_dispatch_or_confirmation() -> anyhow::Result<()> {
    let dispatches = Arc::new(AtomicUsize::new(0));
    let (server_transport, client_transport) = tokio::io::duplex(8192);
    let server = CliMcpServer::with_config(
        counted_write_registry(dispatches.clone()),
        CliMcpServerConfig::default().with_execution_tool_name("repo"),
    )?;
    tokio::spawn(async move {
        server.serve(server_transport).await?.waiting().await?;
        anyhow::Ok(())
    });

    let client = TestClient::new().serve(client_transport).await?;
    let mut dry_run = write_request("Dry")?;
    dry_run.mode = mcp_twill::RunMode::DryRun;
    let result = client
        .call_tool(CallToolRequestParams::new("repo-write").with_arguments(json_object(dry_run)?))
        .await?;
    assert_eq!(result.is_error, Some(false));
    let structured = result.structured_content.unwrap();
    assert_eq!(structured["status"], "ok");
    assert!(structured.get("plan").is_some());
    assert_eq!(structured["plan"]["lane"], "write");
    assert_eq!(dispatches.load(Ordering::SeqCst), 0);

    client.cancel().await?;
    Ok(())
}

#[tokio::test]
async fn changed_args_fail_replay() -> anyhow::Result<()> {
    let dispatches = Arc::new(AtomicUsize::new(0));
    let (server_transport, client_transport) = tokio::io::duplex(8192);
    let server = CliMcpServer::with_config(
        counted_write_registry(dispatches.clone()),
        CliMcpServerConfig::default().with_execution_tool_name("repo"),
    )?;
    tokio::spawn(async move {
        server.serve(server_transport).await?.waiting().await?;
        anyhow::Ok(())
    });

    let client = TestClient::new().serve(client_transport).await?;
    let requested = write_request("Original")?;
    let permission_required = client
        .call_tool(
            CallToolRequestParams::new("repo-write")
                .with_arguments(json_object(requested.clone())?),
        )
        .await?;
    let token = permission_required.structured_content.unwrap()["replay"]["token"]
        .as_str()
        .unwrap()
        .to_string();

    let mut changed = write_request("Changed")?;
    changed.approval = Some(mcp_twill::ApprovalInput {
        token: token.clone(),
        confirm: true,
    });
    let result = client
        .call_tool(CallToolRequestParams::new("repo-write").with_arguments(json_object(changed)?))
        .await?;
    assert_eq!(result.is_error, Some(true));
    let structured = result.structured_content.unwrap();
    assert_eq!(structured["status"], "approvalInvalid");
    assert_eq!(structured["error"]["code"], "approval_invalid");
    assert_eq!(dispatches.load(Ordering::SeqCst), 0);

    let mut approved = requested;
    approved.approval = Some(mcp_twill::ApprovalInput {
        token,
        confirm: true,
    });
    let dispatched = client
        .call_tool(CallToolRequestParams::new("repo-write").with_arguments(json_object(approved)?))
        .await?;
    assert_eq!(dispatched.is_error, Some(false));
    assert_eq!(dispatches.load(Ordering::SeqCst), 1);

    client.cancel().await?;
    Ok(())
}

#[tokio::test]
async fn changed_output_fields_fail_replay() -> anyhow::Result<()> {
    let dispatches = Arc::new(AtomicUsize::new(0));
    let (server_transport, client_transport) = tokio::io::duplex(8192);
    let server = CliMcpServer::with_config(
        counted_write_registry(dispatches.clone()),
        CliMcpServerConfig::default().with_execution_tool_name("repo"),
    )?;
    tokio::spawn(async move {
        server.serve(server_transport).await?.waiting().await?;
        anyhow::Ok(())
    });

    let client = TestClient::new().serve(client_transport).await?;
    let mut requested = write_request("Original")?;
    requested.output = Some(OutputSpec {
        fields: Some(vec!["id".to_string()]),
        ..Default::default()
    });
    let permission_required = client
        .call_tool(CallToolRequestParams::new("repo-write").with_arguments(json_object(requested)?))
        .await?;
    let token = permission_required.structured_content.unwrap()["replay"]["token"]
        .as_str()
        .unwrap()
        .to_string();

    let mut changed = write_request("Original")?;
    changed.output = Some(OutputSpec {
        fields: Some(vec!["title".to_string()]),
        ..Default::default()
    });
    changed.approval = Some(mcp_twill::ApprovalInput {
        token,
        confirm: true,
    });
    let result = client
        .call_tool(CallToolRequestParams::new("repo-write").with_arguments(json_object(changed)?))
        .await?;
    assert_eq!(result.is_error, Some(true));
    let structured = result.structured_content.unwrap();
    assert_eq!(structured["status"], "approvalInvalid");
    assert_eq!(dispatches.load(Ordering::SeqCst), 0);

    client.cancel().await?;
    Ok(())
}

#[tokio::test]
async fn expired_token_fails_replay() -> anyhow::Result<()> {
    let dispatches = Arc::new(AtomicUsize::new(0));
    let (server_transport, client_transport) = tokio::io::duplex(8192);
    let server = CliMcpServer::with_config(
        counted_write_registry(dispatches.clone()),
        CliMcpServerConfig::default()
            .with_execution_tool_name("repo")
            .with_replay_ttl_seconds(-1),
    )?;
    tokio::spawn(async move {
        server.serve(server_transport).await?.waiting().await?;
        anyhow::Ok(())
    });

    let client = TestClient::new().serve(client_transport).await?;
    let requested = write_request("Expired")?;
    let permission_required = client
        .call_tool(
            CallToolRequestParams::new("repo-write")
                .with_arguments(json_object(requested.clone())?),
        )
        .await?;
    let token = permission_required.structured_content.unwrap()["replay"]["token"]
        .as_str()
        .unwrap()
        .to_string();

    let mut approved = requested;
    approved.approval = Some(mcp_twill::ApprovalInput {
        token,
        confirm: true,
    });
    let result = client
        .call_tool(CallToolRequestParams::new("repo-write").with_arguments(json_object(approved)?))
        .await?;
    assert_eq!(result.is_error, Some(true));
    let structured = result.structured_content.unwrap();
    assert_eq!(structured["status"], "approvalInvalid");
    assert_eq!(
        structured["error"]["details"]["reason"],
        "approval token expired"
    );
    assert_eq!(dispatches.load(Ordering::SeqCst), 0);

    client.cancel().await?;
    Ok(())
}

#[tokio::test]
async fn reused_token_fails_replay() -> anyhow::Result<()> {
    let dispatches = Arc::new(AtomicUsize::new(0));
    let (server_transport, client_transport) = tokio::io::duplex(8192);
    let server = CliMcpServer::with_config(
        counted_write_registry(dispatches.clone()),
        CliMcpServerConfig::default().with_execution_tool_name("repo"),
    )?;
    tokio::spawn(async move {
        server.serve(server_transport).await?.waiting().await?;
        anyhow::Ok(())
    });

    let client = TestClient::new().serve(client_transport).await?;
    let requested = write_request("Reuse")?;
    let permission_required = client
        .call_tool(
            CallToolRequestParams::new("repo-write")
                .with_arguments(json_object(requested.clone())?),
        )
        .await?;
    let token = permission_required.structured_content.unwrap()["replay"]["token"]
        .as_str()
        .unwrap()
        .to_string();

    let mut approved = requested.clone();
    approved.approval = Some(mcp_twill::ApprovalInput {
        token: token.clone(),
        confirm: true,
    });
    let dispatched = client
        .call_tool(
            CallToolRequestParams::new("repo-write").with_arguments(json_object(approved.clone())?),
        )
        .await?;
    assert_eq!(dispatched.is_error, Some(false));

    let reused = client
        .call_tool(CallToolRequestParams::new("repo-write").with_arguments(json_object(approved)?))
        .await?;
    assert_eq!(reused.is_error, Some(true));
    let structured = reused.structured_content.unwrap();
    assert_eq!(structured["status"], "approvalInvalid");
    assert_eq!(
        structured["error"]["details"]["reason"],
        "approval token is unknown or already used"
    );
    assert_eq!(dispatches.load(Ordering::SeqCst), 1);

    client.cancel().await?;
    Ok(())
}

#[tokio::test]
async fn dry_run_uses_debug_response_profile() -> anyhow::Result<()> {
    let (server_transport, client_transport) = tokio::io::duplex(8192);
    let server = CliMcpServer::new(registry())?;
    tokio::spawn(async move {
        server.serve(server_transport).await?.waiting().await?;
        anyhow::Ok(())
    });

    let client = TestClient::new().serve(client_transport).await?;
    let request = RunRequest {
        command: "issues list".to_string(),
        args: BTreeMap::new(),
        stdin: None,
        output: None,
        mode: mcp_twill::RunMode::Execute,
        approval: None,
        dry_run: true,
    };
    let result = client
        .call_tool(CallToolRequestParams::new("run").with_arguments(json_object(request)?))
        .await?;
    let structured = result.structured_content.unwrap();
    assert_eq!(structured["status"], "ok");
    assert!(structured.get("plan").is_some());
    assert_eq!(structured["plan"]["commandPath"], json!(["issues", "list"]));

    client.cancel().await?;
    Ok(())
}

#[tokio::test]
async fn text_response_profile_omits_structured_content_on_success() -> anyhow::Result<()> {
    let (server_transport, client_transport) = tokio::io::duplex(8192);
    let server = CliMcpServer::new(registry())?;
    tokio::spawn(async move {
        server.serve(server_transport).await?.waiting().await?;
        anyhow::Ok(())
    });

    let client = TestClient::new().serve(client_transport).await?;
    let request = RunRequest {
        command: "issues list".to_string(),
        args: BTreeMap::new(),
        stdin: None,
        output: Some(OutputSpec {
            profile: Some(ResponseProfile::Text),
            ..Default::default()
        }),
        mode: mcp_twill::RunMode::Execute,
        approval: None,
        dry_run: false,
    };
    let result = client
        .call_tool(CallToolRequestParams::new("run").with_arguments(json_object(request)?))
        .await?;
    assert_eq!(result.is_error, Some(false));
    assert!(result.structured_content.is_none());

    client.cancel().await?;
    Ok(())
}

#[tokio::test]
async fn text_output_format_omits_structured_content_on_success() -> anyhow::Result<()> {
    let (server_transport, client_transport) = tokio::io::duplex(8192);
    let server = CliMcpServer::new(registry())?;
    tokio::spawn(async move {
        server.serve(server_transport).await?.waiting().await?;
        anyhow::Ok(())
    });

    let client = TestClient::new().serve(client_transport).await?;
    let request = RunRequest {
        command: "issues list".to_string(),
        args: BTreeMap::new(),
        stdin: None,
        output: Some(OutputSpec {
            format: OutputFormat::Text,
            ..Default::default()
        }),
        mode: mcp_twill::RunMode::Execute,
        approval: None,
        dry_run: false,
    };
    let result = client
        .call_tool(CallToolRequestParams::new("run").with_arguments(json_object(request)?))
        .await?;
    assert_eq!(result.is_error, Some(false));
    assert!(result.structured_content.is_none());
    assert!(serde_json::to_string(&result)?.contains("One"));

    client.cancel().await?;
    Ok(())
}

fn workspace_registry() -> CommandRegistry {
    registry()
        .declare_workspace(mcp_twill::WorkspaceDecl::file(
            "repo",
            "file:///declared/repo",
        ))
        .register(
            CommandSpec::new(["files", "read"], "Read file", "Read a repository file")
                .with_arg(mcp_twill::ArgSpec::path("path", "File to read", "repo"))
                .with_permission(PermissionSpec::new(
                    PermissionEffect::Read,
                    "repo",
                    "Reads repository files",
                )),
            |context: mcp_twill::CommandContext| async move {
                Ok(CommandOutput::structured(
                    json!({ "planned": context.plan.workspace_roots }),
                ))
            },
        )
}

// End-to-end over the real protocol: a client that declares the roots
// capability serves roots/list, and the selected MCP root (not the declared
// fallback) governs planning and containment.
#[tokio::test]
async fn client_roots_resolve_workspaces_over_mcp() -> anyhow::Result<()> {
    let (server_transport, client_transport) = tokio::io::duplex(8192);
    let server = CliMcpServer::new(workspace_registry())?;
    tokio::spawn(async move {
        server.serve(server_transport).await?.waiting().await?;
        anyhow::Ok(())
    });

    let client = RootsClient {
        roots: vec![rmcp::model::Root::new("file:///client/repo").with_name("repo")],
    }
    .serve(client_transport)
    .await?;

    // Dry run: the plan must show the client root, its source, and the
    // selection reason.
    let mut dry_run = request(
        "files read $args.path",
        json!({ "path": "file:///client/repo/src/lib.rs" }),
    )?;
    dry_run.dry_run = true;
    let result = client
        .call_tool(CallToolRequestParams::new("run").with_arguments(json_object(dry_run)?))
        .await?;
    assert_eq!(result.is_error, Some(false));
    let structured = result
        .structured_content
        .clone()
        .expect("structured envelope");
    let root = &structured["plan"]["workspaceRoots"][0];
    assert_eq!(root["id"], json!("repo"));
    assert_eq!(root["rootUri"], json!("file:///client/repo"));
    assert_eq!(root["source"], json!("mcp_roots"));
    assert_eq!(root["selectionReason"], json!("matched_by_name"));

    // A path inside the declared fallback but outside the client root is
    // rejected: client roots outrank the declared workspace.
    let outside = request(
        "files read $args.path",
        json!({ "path": "file:///declared/repo/src/lib.rs" }),
    )?;
    let result = client
        .call_tool(CallToolRequestParams::new("run").with_arguments(json_object(outside)?))
        .await?;
    assert_eq!(result.is_error, Some(true));
    let structured = result
        .structured_content
        .clone()
        .expect("structured envelope");
    assert_eq!(structured["error"]["code"], json!("workspace_mismatch"));
    assert_eq!(
        structured["error"]["details"]["selectedRoot"],
        json!("file:///client/repo")
    );

    // A path inside the client root executes, and the handler sees the
    // resolved root on its plan.
    let inside = request(
        "files read $args.path",
        json!({ "path": "file:///client/repo/src/lib.rs" }),
    )?;
    let result = client
        .call_tool(CallToolRequestParams::new("run").with_arguments(json_object(inside)?))
        .await?;
    assert_eq!(result.is_error, Some(false));
    let structured = result
        .structured_content
        .clone()
        .expect("structured envelope");
    assert_eq!(
        structured["output"]["structured"]["planned"][0]["rootUri"],
        json!("file:///client/repo")
    );

    client.cancel().await?;
    Ok(())
}

// A client without the roots capability falls back to the declared workspace.
#[tokio::test]
async fn declared_fallback_governs_when_client_has_no_roots() -> anyhow::Result<()> {
    let (server_transport, client_transport) = tokio::io::duplex(8192);
    let server = CliMcpServer::new(workspace_registry())?;
    tokio::spawn(async move {
        server.serve(server_transport).await?.waiting().await?;
        anyhow::Ok(())
    });

    let client = TestClient::new().serve(client_transport).await?;
    let mut dry_run = request(
        "files read $args.path",
        json!({ "path": "file:///declared/repo/src/lib.rs" }),
    )?;
    dry_run.dry_run = true;
    let result = client
        .call_tool(CallToolRequestParams::new("run").with_arguments(json_object(dry_run)?))
        .await?;
    assert_eq!(result.is_error, Some(false));
    let structured = result
        .structured_content
        .clone()
        .expect("structured envelope");
    let root = &structured["plan"]["workspaceRoots"][0];
    assert_eq!(root["rootUri"], json!("file:///declared/repo"));
    assert_eq!(root["source"], json!("declared"));
    assert_eq!(root["selectionReason"], json!("declared_observation"));

    client.cancel().await?;
    Ok(())
}
