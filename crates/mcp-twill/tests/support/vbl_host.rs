//! RFC 0019's test-only VBL host-profile declarations.

use mcp_twill_host::{
    HostAdapterProfile, HostApplicationRejection, HostConfirmationPolicy, HostConfirmationTrigger,
    HostContextReason, HostGuidanceProjection, HostGuidanceSegment, HostInvocationLimits,
    HostProcessLimits, UnsupportedContextPolicy, VsCodeEngineRange, VsCodeVersion,
};

use crate::vbl::OPERATION_MAPPING;

pub fn vscode_host_profile(
    surface: &mcp_twill::NativeToolSurfaceSnapshot,
) -> mcp_twill::Result<HostAdapterProfile> {
    let start_guidance = "Backed by Visible Browser Lab's shared broker surface. VS Code chat supplies conversation identity out of band, so this compatibility entry point reuses the current ambient session; its model-visible schema and result never accept or expose a session handle. For normal work, call browser operations directly. The tool returns structured JSON success values or structured browser errors with recovery guidance.";
    let operation_suffixes = OPERATION_MAPPING
        .iter()
        .map(|(released, path, _)| {
            (
                path.replace(' ', "."),
                if *released == "start_session" {
                    vec![HostGuidanceSegment::Text(start_guidance.to_string())]
                } else {
                    vec![
                        HostGuidanceSegment::Text(
                            "Backed by Visible Browser Lab's shared broker surface. VS Code chat supplies conversation identity out of band; never call "
                                .to_string(),
                        ),
                        HostGuidanceSegment::Operation {
                            operation_id: "session.start".to_string(),
                        },
                        HostGuidanceSegment::Text(
                            " or invent, request, or pass a session handle. If the host returns session_required, report its recovery guidance. Use only tab_id values owned by the selected session. The tool returns structured JSON success values or structured browser errors with recovery guidance."
                                .to_string(),
                        ),
                    ]
                },
            )
        })
        .collect();
    let unsupported_summary = "VS Code did not expose a compatible chat session resource; Visible Browser Lab requires VS Code 1.120 or newer with the supported invocation-token shape";
    let unsupported = [
        HostContextReason::UnknownTokenShape,
        HostContextReason::InvalidSessionResource,
        HostContextReason::InvalidWorkingDirectory,
        HostContextReason::ProviderFailed,
    ]
    .into_iter()
    .fold(
        UnsupportedContextPolicy::new().allow("application.help"),
        |policy, reason| policy.reason(reason, unsupported_summary),
    )
    .recover_by(
        "update_or_use_explicit_surface",
        "update and reload VS Code, or use the explicit MCP/CLI surface",
    );

    HostAdapterProfile::vscode("vbl-vscode-host", VsCodeVersion::new(1, 120, 0))
        .tool_name_prefix("visible_browser_lab_")
        .icon("$(browser)")
        .guidance(HostGuidanceProjection {
            operation_suffixes,
            ..HostGuidanceProjection::default()
        })
        .prompt_reference("application.help", "vbl")
        .prompt_reference("page.snapshot", "vbl_snapshot")
        .prompt_reference("page.screenshot", "vbl_screenshot")
        .prompt_reference("page.navigate", "vbl_navigate")
        .confirmation(HostConfirmationPolicy::trusted_vscode_ui(
            HostConfirmationTrigger::DeclaredPresentation,
            VsCodeEngineRange::inclusive(
                VsCodeVersion::new(1, 120, 0),
                VsCodeVersion::new(1, 128, 0),
            ),
        ))
        .omit_result_property("session.start", "agent_session_id")
        .unsupported_context(unsupported)
        .absent_context_rejects(
            "session.start",
            HostApplicationRejection::new("session_required")
                .runtime_message(
                    "Global VS Code tool invocations have no conversation identity and do not expose explicit session handles",
                )
                .recover_by(
                    "use_chat_or_explicit_surface",
                    "invoke Visible Browser Lab from a supported VS Code chat, or use the explicit MCP/CLI surface",
                ),
        )
        .invocation_limits(HostInvocationLimits::new(1_048_576, 1_048_576))
        .process_envelope(
            "bin/visible-browser-lab-mcp",
            ["host", "call"],
            HostProcessLimits::new(1_048_576, 2_000),
        )
        .build(surface)
}
