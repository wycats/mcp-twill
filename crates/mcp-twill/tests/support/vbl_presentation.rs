//! Authored RFC 0018 declarations corresponding to VBL's frozen portable
//! presentation observations.

use mcp_twill::{
    ArgSpec, ArgumentRendering, CommandOutput, CommandRegistry, CommandSpec, ConfirmationMessage,
    ConfirmationPredicate, ConfirmationPresentation, PermissionSpec,
};
use serde_json::json;

pub const OPERATION_MAPPING: [(&str, &str, &str); 12] = [
    ("start_session", "session start", "Start Session"),
    ("snapshot", "page snapshot", "Snapshot"),
    ("screenshot", "page screenshot", "Screenshot"),
    ("navigate", "page navigate", "Navigate"),
    ("click", "page click", "Click"),
    ("fill", "form fill", "Fill"),
    ("fill_form", "form fill many", "Fill Form"),
    ("wait_for", "page wait", "Wait For"),
    ("claim_tab", "tabs claim", "Claim Tab"),
    ("close_tab", "tabs close", "Close Tab"),
    ("release_tab", "tabs release", "Release Tab"),
    ("focus_tab", "tabs focus", "Focus Tab"),
];

pub fn registry() -> CommandRegistry {
    let invocation = [
        ("start_session", "Starting a visible browser session"),
        ("snapshot", "Capturing a browser snapshot"),
        ("screenshot", "Capturing a browser screenshot"),
        ("navigate", "Navigating the owned browser tab"),
        ("click", "Clicking a browser element"),
        ("fill", "Filling browser form controls"),
        ("fill_form", "Filling browser form controls"),
        ("wait_for", "Waiting for browser state"),
    ]
    .into_iter()
    .collect::<std::collections::BTreeMap<_, _>>();
    let mut registry = CommandRegistry::new(
        "vbl-presentation",
        "Visible Browser Lab presentation adoption fixture",
    );
    for (method, path, title) in OPERATION_MAPPING {
        let mut spec = CommandSpec::new(
            path.split_whitespace(),
            title,
            format!("RFC 0018 VBL presentation adoption for `{method}`."),
        );
        spec.invocation_message = invocation.get(method).map(|message| (*message).to_string());
        match method {
            "claim_tab" => {
                spec = spec
                    .with_permission(PermissionSpec::write("browser", "Claim a browser tab"))
                    .with_arg(ArgSpec::string("target_id", "Browser target id").optional());
                spec.confirmation = Some(ConfirmationPresentation::new(
                    ConfirmationMessage::new("Claim browser tab?")
                        .text("Claim target ")
                        .argument("target_id", ArgumentRendering::Plain, "(unknown target)")
                        .text(" for this agent session."),
                ));
            }
            "close_tab" => {
                spec = spec
                    .with_permission(PermissionSpec::write("browser", "Close a browser tab"))
                    .with_arg(ArgSpec::string("tab_id", "Owned tab id"));
                spec.confirmation = Some(ConfirmationPresentation::new(
                    ConfirmationMessage::new("Close browser tab?")
                        .text("Close owned tab ")
                        .argument("tab_id", ArgumentRendering::Plain, "(unknown tab)")
                        .text("."),
                ));
            }
            "release_tab" => {
                spec = spec
                    .with_permission(PermissionSpec::write("browser", "Release a browser tab"))
                    .with_arg(ArgSpec::string("tab_id", "Owned tab id"))
                    .with_arg(
                        ArgSpec::inline_schema(
                            "leave_visible",
                            json!({ "type": "boolean" }),
                            "Whether to preserve the tab",
                        )
                        .optional(),
                    )
                    .with_arg(
                        ArgSpec::string("user_instruction", "User preservation instruction")
                            .optional(),
                    );
                spec.confirmation = Some(
                    ConfirmationPresentation::new(
                        ConfirmationMessage::new("Release browser tab?")
                            .text("Release owned tab ")
                            .argument("tab_id", ArgumentRendering::Plain, "(unknown tab)")
                            .text("; a VBL-created target remains eligible for expiry cleanup."),
                    )
                    .case(
                        ConfirmationPredicate::argument_equals("leave_visible", true),
                        ConfirmationMessage::new("Leave browser tab visible?")
                            .text("Release owned tab ")
                            .argument("tab_id", ArgumentRendering::Plain, "(unknown tab)")
                            .text(" and preserve it after this session expires. User instruction: ")
                            .argument(
                                "user_instruction",
                                ArgumentRendering::TrimmedJsonString,
                                "(missing; this request will be rejected)",
                            )
                            .text("."),
                    ),
                );
            }
            "focus_tab" => {
                spec = spec
                    .with_permission(PermissionSpec::write("browser", "Focus a browser tab"))
                    .with_arg(ArgSpec::string("tab_id", "Owned tab id"));
                spec.confirmation = Some(ConfirmationPresentation::new(
                    ConfirmationMessage::new("Bring Chrome forward?")
                        .text("Focus owned tab ")
                        .argument("tab_id", ArgumentRendering::Plain, "(unknown tab)")
                        .text(" for manual inspection or handoff."),
                ));
            }
            _ => {}
        }
        registry = registry.register(spec, |_context| async {
            Ok(CommandOutput::structured(json!({})))
        });
    }
    registry
}
