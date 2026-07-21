use std::sync::Arc;

use mcp_twill::*;
use serde_json::json;

struct ExternalStore;

impl TaskStore for ExternalStore {
    fn scope(&self) -> TaskStoreScope {
        TaskStoreScope::ServerInstance
    }

    fn create(
        &self,
        key: TaskStorageKey,
        record: StoredTaskRecord,
    ) -> TaskStoreFuture<'_, std::result::Result<TaskStoreCreate, TaskStoreError>> {
        let _ = (
            key.as_bytes(),
            record.revision().get(),
            record.expires_at().map(TaskExpiration::unix_millis),
            record.storage_bytes(),
        );
        Box::pin(async {
            Err(TaskStoreError::new(std::io::Error::other(
                "private backend",
            )))
        })
    }

    fn get(
        &self,
        _key: TaskStorageKey,
    ) -> TaskStoreFuture<'_, std::result::Result<Option<StoredTaskRecord>, TaskStoreError>> {
        Box::pin(async { Ok(None) })
    }

    fn compare_and_set(
        &self,
        _key: TaskStorageKey,
        _expected: TaskRevision,
        next: StoredTaskRecord,
    ) -> TaskStoreFuture<'_, std::result::Result<TaskStoreWrite, TaskStoreError>> {
        let _ = next.into_storage_bytes();
        Box::pin(async { Ok(TaskStoreWrite::Missing) })
    }

    fn remove(
        &self,
        _key: TaskStorageKey,
    ) -> TaskStoreFuture<'_, std::result::Result<TaskStoreRemoval, TaskStoreError>> {
        Box::pin(async { Ok(TaskStoreRemoval::Missing) })
    }
}

fn object_contract() -> OutputContract {
    OutputContract {
        application: Some(ApplicationResultContract::new(json!({
            "type": "object",
            "properties": { "ok": { "type": "boolean" } },
            "required": ["ok"],
            "additionalProperties": false
        }))),
        ..OutputContract::default()
    }
}

fn registry(support: TaskSupportSpec) -> CommandRegistry {
    let spec = CommandSpec::new(["work"], "Work", "Perform work")
        .task_support(support)
        .with_output(object_contract());
    CommandRegistry::new("tasks", "Task delivery tests").register_dynamic(spec, |_| async {
        Ok::<_, DynamicCommandFailure>(ApplicationSuccess::value(json!({ "ok": true })))
    })
}

fn surface(
    registry: &CommandRegistry,
    target: McpProtocolTarget,
    delivery: TaskDeliveryDecl,
) -> mcp_twill::Result<NativeToolSurface> {
    NativeToolSurface::builder("task-tools")
        .framework_help(FrameworkHelpProjection::Omitted)
        .confirmation_route(NativeConfirmationRoute::Unavailable)
        .task_delivery(delivery)
        .direct("work", "work")
        .build(registry, target)
}

#[test]
fn delivery_declarations_and_compiled_views_use_the_accepted_contract() -> anyhow::Result<()> {
    assert_eq!(
        serde_json::to_value(TaskDeliveryDecl::default())?,
        json!({ "kind": "disabled" })
    );
    assert_eq!(
        serde_json::to_value(TaskDeliveryDecl::Legacy2025_11_25)?,
        json!({ "kind": "legacy2025_11_25" })
    );
    assert_eq!(
        serde_json::to_value(TaskDeliveryDecl::tasks_extension(
            ExtensionOptionalPolicy::DeferredWhenAvailable,
            3_600_000,
        ))?,
        json!({
            "kind": "tasksExtension",
            "optionalPolicy": "deferredWhenAvailable",
            "retentionMs": 3_600_000
        })
    );

    let registry = registry(TaskSupportSpec::Optional);
    let legacy = surface(
        &registry,
        McpProtocolTarget::V2025_11_25,
        TaskDeliveryDecl::Legacy2025_11_25,
    )?;
    let CompiledTaskDelivery::Legacy2025_11_25(compiled) = legacy.snapshot().task_delivery() else {
        anyhow::bail!("expected legacy delivery");
    };
    assert!(!compiled.capability().supports_list());
    assert!(compiled.capability().supports_cancel());
    assert_eq!(compiled.runtime_contract_version(), 1);
    assert_eq!(compiled.max_stored_task_record_bytes(), 1_048_576);
    assert_eq!(compiled.max_stored_tasks(), 256);
    assert_eq!(
        legacy.snapshot().document()["taskDelivery"],
        json!({
            "kind": "legacy2025_11_25",
            "runtimeContractVersion": 1,
            "maxStoredTaskRecordBytes": 1_048_576,
            "maxStoredTasks": 256
        })
    );

    let extension = surface(
        &registry,
        McpProtocolTarget::V2026_07_28,
        TaskDeliveryDecl::tasks_extension(
            ExtensionOptionalPolicy::DeferredWhenAvailable,
            3_600_000,
        ),
    )?;
    let CompiledTaskDelivery::TasksExtension(compiled) = extension.snapshot().task_delivery()
    else {
        anyhow::bail!("expected extension delivery");
    };
    assert_eq!(compiled.extension_id(), "io.modelcontextprotocol/tasks");
    assert_eq!(compiled.retention_ms(), 3_600_000);
    assert_eq!(compiled.runtime_contract_version(), 1);
    assert_ne!(
        legacy.snapshot().surface_hash(),
        extension.snapshot().surface_hash()
    );
    Ok(())
}

#[test]
fn delivery_and_server_finalization_fail_closed() -> anyhow::Result<()> {
    let required = registry(TaskSupportSpec::Required);
    let error = surface(
        &required,
        McpProtocolTarget::V2026_07_28,
        TaskDeliveryDecl::Disabled,
    )
    .unwrap_err();
    assert!(error.to_string().contains("disabled task delivery"));

    let optional = registry(TaskSupportSpec::Optional);
    for retention in [0, 604_800_001] {
        let error = surface(
            &optional,
            McpProtocolTarget::V2026_07_28,
            TaskDeliveryDecl::tasks_extension(ExtensionOptionalPolicy::Immediate, retention),
        )
        .unwrap_err();
        assert!(error.to_string().contains("retention"));
    }
    let error = surface(
        &optional,
        McpProtocolTarget::V2025_11_25,
        TaskDeliveryDecl::tasks_extension(ExtensionOptionalPolicy::Immediate, 1),
    )
    .unwrap_err();
    assert!(error.to_string().contains("2026-07-28"));

    let extension = surface(
        &optional,
        McpProtocolTarget::V2026_07_28,
        TaskDeliveryDecl::tasks_extension(ExtensionOptionalPolicy::Immediate, 1_000),
    )?;
    let missing = match CliMcpServer::with_surface(optional.clone(), extension.clone()) {
        Ok(_) => anyhow::bail!("expected a missing runtime failure"),
        Err(error) => error,
    };
    assert!(missing.to_string().contains("explicit task runtime"));
    let wrong_scope = match CliMcpServer::builder(optional.clone())
        .surface(extension.clone())
        .task_runtime(
            InMemoryTaskStore::connection(),
            TaskAccessPolicy::CapabilityId,
        )
        .build()
    {
        Ok(_) => anyhow::bail!("expected a scope failure"),
        Err(error) => error,
    };
    assert!(wrong_scope.to_string().contains("server-instance"));
    let repeated = match CliMcpServer::builder(optional.clone())
        .surface(extension.clone())
        .task_runtime(
            InMemoryTaskStore::server_instance(),
            TaskAccessPolicy::CapabilityId,
        )
        .task_runtime(
            InMemoryTaskStore::server_instance(),
            TaskAccessPolicy::CapabilityId,
        )
        .build()
    {
        Ok(_) => anyhow::bail!("expected a repeated runtime failure"),
        Err(error) => error,
    };
    assert!(repeated.to_string().contains("more than once"));
    let server = CliMcpServer::builder(optional)
        .surface(extension)
        .task_runtime(
            InMemoryTaskStore::server_instance(),
            TaskAccessPolicy::CapabilityId,
        )
        .build()?;
    assert!(matches!(
        server.into_stateless_service(),
        Err(FrameworkError::ProtocolReleaseUnsealed)
    ));

    let registry = registry(TaskSupportSpec::Optional);
    let extension = surface(
        &registry,
        McpProtocolTarget::V2026_07_28,
        TaskDeliveryDecl::tasks_extension(ExtensionOptionalPolicy::Immediate, 1_000),
    )?;
    let shared: Arc<dyn TaskStore> = InMemoryTaskStore::server_instance();
    let first = CliMcpServer::builder(registry.clone())
        .surface(extension.clone())
        .task_runtime(shared.clone(), TaskAccessPolicy::CapabilityId)
        .build()?;
    let second = match CliMcpServer::builder(registry.clone())
        .surface(extension.clone())
        .task_runtime(shared.clone(), TaskAccessPolicy::CapabilityId)
        .build()
    {
        Ok(_) => anyhow::bail!("expected an exclusive-mount failure"),
        Err(error) => error,
    };
    assert!(second.to_string().contains("already mounted"));
    drop(first);
    CliMcpServer::builder(registry)
        .surface(extension)
        .task_runtime(shared, TaskAccessPolicy::CapabilityId)
        .build()?;
    Ok(())
}

#[tokio::test]
async fn in_memory_stores_are_scoped_bounded_and_atomic() -> anyhow::Result<()> {
    assert_eq!(
        InMemoryTaskStore::connection().scope(),
        TaskStoreScope::Connection
    );
    assert_eq!(
        InMemoryTaskStore::server_instance().scope(),
        TaskStoreScope::ServerInstance
    );

    // External stores receive only opaque keys and validated bytes. The
    // framework itself exercises the full semantic codec through lifecycle
    // tests; this public test proves redaction and constructor boundaries.
    let backend = TaskStoreError::new(std::io::Error::other("private backend"));
    assert_eq!(backend.to_string(), "task store failed");
    assert_eq!(format!("{backend:?}"), "TaskStoreError(<redacted>)");
    assert!(std::error::Error::source(&backend).is_none());

    let access = TaskAccessScope::new(b"principal")?;
    assert_eq!(format!("{access:?}"), "TaskAccessScope(<redacted>)");
    assert!(TaskAccessScope::new([]).is_err());
    assert!(TaskAccessScope::new(vec![0; 4097]).is_err());
    Ok(())
}

#[test]
fn runtime_constants_and_public_store_trait_are_usable() {
    fn accepts_store(_: Arc<dyn TaskStore>) {}
    accepts_store(InMemoryTaskStore::server_instance());
    accepts_store(Arc::new(ExternalStore));
    assert_eq!(TASK_RUNTIME_CONTRACT_VERSION, 1);
    assert_eq!(MAX_STORED_TASK_RECORD_BYTES, 1_048_576);
    assert_eq!(MAX_STORED_TASKS, 256);
    let codec = StoredTaskRecord::from_storage_bytes([0xff]).unwrap_err();
    assert_eq!(codec.to_string(), "task record codec failed");
    assert_eq!(format!("{codec:?}"), "TaskRecordCodecError(<redacted>)");
}

#[test]
fn omitted_and_explicit_disabled_are_identical() -> anyhow::Result<()> {
    let registry = registry(TaskSupportSpec::Optional);
    let omitted = NativeToolSurface::builder("task-tools")
        .framework_help(FrameworkHelpProjection::Omitted)
        .confirmation_route(NativeConfirmationRoute::Unavailable)
        .direct("work", "work")
        .build(&registry, McpProtocolTarget::V2026_07_28)?;
    let explicit = surface(
        &registry,
        McpProtocolTarget::V2026_07_28,
        TaskDeliveryDecl::Disabled,
    )?;
    assert_eq!(omitted.declaration(), explicit.declaration());
    assert_eq!(
        omitted.snapshot().canonical_json(),
        explicit.snapshot().canonical_json()
    );
    assert!(omitted.snapshot().document().get("taskDelivery").is_none());
    Ok(())
}
