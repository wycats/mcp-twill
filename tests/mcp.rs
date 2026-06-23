use std::collections::BTreeMap;

use mcp_twill::{
    CliMcpServer, CommandOutput, CommandRegistry, CommandSpec, PermissionEffect, PermissionSpec,
    RunRequest,
};
use futures::StreamExt;
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

#[tokio::test]
async fn mcp_exposes_two_tools_and_resources_prompts() -> anyhow::Result<()> {
    let (server_transport, client_transport) = tokio::io::duplex(8192);
    let server = CliMcpServer::new(registry());
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

    let resources = client.list_resources(Default::default()).await?;
    assert!(
        resources
            .resources
            .iter()
            .any(|r| r.uri == "cli://commands")
    );
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

    client.cancel().await?;
    server_handle.await??;
    Ok(())
}

#[tokio::test]
async fn mcp_run_emits_progress_and_returns_structured_content() -> anyhow::Result<()> {
    let (server_transport, client_transport) = tokio::io::duplex(8192);
    let server = CliMcpServer::new(registry());
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
    assert!(value.to_string().contains("issues"));
    assert!(seen.iter().any(|message| message.contains("Parsing")));
    assert!(seen.iter().any(|message| message.contains("complete")));

    client.cancel().await?;
    Ok(())
}

#[tokio::test]
async fn task_augmented_run_completes_when_negotiated() -> anyhow::Result<()> {
    let (server_transport, client_transport) = tokio::io::duplex(8192);
    let server = CliMcpServer::new(registry());
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
