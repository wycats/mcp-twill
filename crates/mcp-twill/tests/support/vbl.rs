//! Authored Twill adoption declarations for the pinned VBL observations.
//!
//! This module is intentionally test-only. Nothing here is deserialized from
//! the released-observation bundle: these declarations are new Twill input
//! whose projections can be compared with what VBL v0.4.9 shipped. This is the
//! RFC 0011 guidance slice, not the later complete application schema.

use mcp_twill::{
    ArgSpec, CommandExample, CommandOutput, CommandRegistry, CommandSpec, Field, TypeDecl, Variant,
};
use serde_json::Value;
use serde_json::json;

pub const PREAMBLE: &str =
    "Routine actions attach to the owned target and preserve the user's active application.";

/// RFC 0014's authored ownership reconciliation for the released VBL error
/// inventory. The fixture is evidence; these assignments are Twill design.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorOwner {
    Framework(&'static str),
    Application,
}

pub const ERROR_OWNERS: [(&str, ErrorOwner); 22] = [
    ("chrome_unavailable", ErrorOwner::Application),
    (
        "invalid_request_context",
        ErrorOwner::Framework("invalid_request_context"),
    ),
    ("session_required", ErrorOwner::Application),
    ("unknown_session", ErrorOwner::Application),
    ("session_expired", ErrorOwner::Application),
    ("unknown_tab", ErrorOwner::Application),
    ("tab_not_owned", ErrorOwner::Application),
    ("tab_not_active", ErrorOwner::Application),
    ("target_missing", ErrorOwner::Application),
    ("target_owned", ErrorOwner::Application),
    ("invalid_input", ErrorOwner::Application),
    ("operation_timeout", ErrorOwner::Application),
    ("focus_required", ErrorOwner::Application),
    ("element_not_found", ErrorOwner::Application),
    ("element_ambiguous", ErrorOwner::Application),
    ("element_stale", ErrorOwner::Application),
    ("element_not_actionable", ErrorOwner::Application),
    ("artifact_not_found", ErrorOwner::Application),
    ("artifact_error", ErrorOwner::Application),
    ("workspace_unavailable", ErrorOwner::Application),
    ("workspace_context_conflict", ErrorOwner::Application),
    ("path_outside_workspace", ErrorOwner::Application),
];

/// Released baseline operation name, authored Twill path, and display title.
///
/// The later RFC 0015 compiler groups these 63 operations into the 27-tool
/// served surface. This fixture deliberately stops before that translation.
pub const OPERATION_MAPPING: [(&str, &str, &str); 63] = [
    ("start_session", "session start", "Start Session"),
    ("list_tabs", "tabs list", "List Tabs"),
    ("new_tab", "tabs new", "New Tab"),
    ("claim_tab", "tabs claim", "Claim Tab"),
    ("release_tab", "tabs release", "Release Tab"),
    ("focus_tab", "tabs focus", "Focus Tab"),
    ("close_tab", "tabs close", "Close Tab"),
    ("snapshot", "page snapshot", "Snapshot"),
    ("navigate", "page navigate", "Navigate"),
    ("wait_for", "page wait", "Wait For"),
    ("click", "page click", "Click"),
    ("fill", "form fill", "Fill"),
    ("fill_form", "form fill many", "Fill Form"),
    ("type_text", "input type text", "Type Text"),
    ("press_key", "input press key", "Press Key"),
    ("screenshot", "page screenshot", "Screenshot"),
    ("evaluate", "page evaluate", "Evaluate"),
    (
        "interact_select_options",
        "interact select options",
        "Specialized Interaction: Select Options",
    ),
    (
        "interact_set_checked",
        "interact set checked",
        "Specialized Interaction: Set Checked",
    ),
    (
        "interact_hover",
        "interact hover",
        "Specialized Interaction: Hover",
    ),
    (
        "interact_drag",
        "interact drag",
        "Specialized Interaction: Drag",
    ),
    (
        "interact_drop",
        "interact drop",
        "Specialized Interaction: Drop",
    ),
    (
        "interact_upload_files",
        "interact upload files",
        "Specialized Interaction: Upload Files",
    ),
    (
        "interact_handle_dialog",
        "interact handle dialog",
        "Specialized Interaction: Handle Dialog",
    ),
    (
        "interact_scroll",
        "interact scroll",
        "Specialized Interaction: Scroll",
    ),
    (
        "interact_click_at",
        "interact click at",
        "Specialized Interaction: Click At",
    ),
    ("console_list", "console list", "Console Diagnostics: List"),
    ("console_get", "console get", "Console Diagnostics: Get"),
    (
        "console_clear",
        "console clear",
        "Console Diagnostics: Clear",
    ),
    ("network_list", "network list", "Network Diagnostics: List"),
    ("network_get", "network get", "Network Diagnostics: Get"),
    (
        "network_clear",
        "network clear",
        "Network Diagnostics: Clear",
    ),
    (
        "emulation_set_viewport",
        "emulation set viewport",
        "Emulation: Set Viewport",
    ),
    (
        "emulation_set_network",
        "emulation set network",
        "Emulation: Set Network",
    ),
    (
        "emulation_set_cpu",
        "emulation set cpu",
        "Emulation: Set Cpu",
    ),
    (
        "emulation_set_geolocation",
        "emulation set geolocation",
        "Emulation: Set Geolocation",
    ),
    (
        "emulation_set_media",
        "emulation set media",
        "Emulation: Set Media",
    ),
    (
        "emulation_set_user_agent",
        "emulation set user agent",
        "Emulation: Set User Agent",
    ),
    (
        "emulation_set_headers",
        "emulation set headers",
        "Emulation: Set Headers",
    ),
    ("emulation_reset", "emulation reset", "Emulation: Reset"),
    (
        "performance_start_trace",
        "performance start trace",
        "Performance: Start Trace",
    ),
    (
        "performance_stop_trace",
        "performance stop trace",
        "Performance: Stop Trace",
    ),
    (
        "performance_vitals",
        "performance vitals",
        "Performance: Vitals",
    ),
    (
        "performance_analyze",
        "performance analyze",
        "Performance: Analyze",
    ),
    ("audit_run", "audit run", "Audit: Run"),
    ("memory_capture", "memory capture", "Memory: Capture"),
    ("memory_summary", "memory summary", "Memory: Summary"),
    ("memory_classes", "memory classes", "Memory: Classes"),
    ("memory_node", "memory node", "Memory: Node"),
    (
        "memory_dominators",
        "memory dominators",
        "Memory: Dominators",
    ),
    ("memory_retainers", "memory retainers", "Memory: Retainers"),
    (
        "memory_retaining_paths",
        "memory retaining paths",
        "Memory: Retaining Paths",
    ),
    ("memory_edges", "memory edges", "Memory: Edges"),
    ("memory_close", "memory close", "Memory: Close"),
    ("screencast_start", "screencast start", "Screencast: Start"),
    ("screencast_stop", "screencast stop", "Screencast: Stop"),
    (
        "screencast_status",
        "screencast status",
        "Screencast: Status",
    ),
    ("artifacts_list", "artifacts list", "Artifacts: List"),
    (
        "artifacts_metadata",
        "artifacts metadata",
        "Artifacts: Metadata",
    ),
    ("artifacts_read", "artifacts read", "Artifacts: Read"),
    ("artifacts_export", "artifacts export", "Artifacts: Export"),
    ("artifacts_delete", "artifacts delete", "Artifacts: Delete"),
    ("help", "application help", "Browser Tool Help"),
];

pub fn registry() -> CommandRegistry {
    let mut registry = CommandRegistry::new("vbl-adoption", "Visible Browser Lab adoption fixture")
        .declare_preamble(PREAMBLE)
        .declare_type(
            TypeDecl::union("element-target", "How to identify a page element")
                .variant(
                    Variant::new("reference", "Use a reference from the latest page snapshot")
                        .field(Field::string("ref", "Snapshot element reference")),
                )
                .variant(
                    Variant::new("css", "Use a CSS selector escape hatch")
                        .field(Field::string("css", "CSS selector"))
                        .fallback("the snapshot cannot represent the target"),
                ),
        );

    for (released_name, path, title) in OPERATION_MAPPING {
        let mut spec = CommandSpec::new(
            path.split_whitespace(),
            title,
            format!("Authored Twill adoption declaration for VBL `{released_name}`."),
        );
        spec = match released_name {
            "snapshot" => spec.use_when("inspecting an unfamiliar page before acting"),
            "fill" => spec
                .use_when("filling a single ordinary field")
                .alternative("form fill many", "updating two or more controls in one pass"),
            "fill_form" => spec
                .use_when(
                    "updating two or more controls, including combined select and checkbox changes",
                )
                .alternative("form fill", "filling a single ordinary field"),
            "type_text" => {
                spec.use_when("typing into contenteditable controls or at an established caret")
            }
            "press_key" => spec.use_when(
                "sending named keys or shortcuts to a resolved element, or targetless input after the owned document has browser focus",
            ),
            "wait_for" => spec.use_when("waiting for asynchronous page state"),
            "screenshot" => spec.use_when("inspecting visual appearance"),
            "console_list" | "console_get" | "console_clear" =>
                spec.use_when("diagnosing runtime console behavior"),
            "network_list" | "network_get" | "network_clear" =>
                spec.use_when("diagnosing runtime network behavior"),
            "help" => spec.use_when("selecting an operation in a specialized domain"),
            "focus_tab" => spec.use_when(
                "bringing Chrome forward for manual inspection, handoff, or targetless input",
            ),
            "click" => {
                let mut example = CommandExample::new(
                    "page click --target $args.target",
                    "Click an element by its latest snapshot reference",
                );
                example
                    .args
                    .insert("target".to_string(), json!({"ref": "e1"}));
                spec.with_arg(ArgSpec::named(
                    "target",
                    "element-target",
                    "Element to click",
                ))
                .with_example(example)
            }
            "interact_click_at" => spec.use_when(
                "using a specialized interaction, with targetless click_at only after the owned document has browser focus",
            ),
            "evaluate" => spec.fallback(
                [
                    "page snapshot",
                    "console list",
                    "network list",
                ],
                "semantic snapshots and diagnostics do not expose the state you need",
            ),
            _ => spec,
        };
        registry = registry.register(spec, |_context| async {
            Ok(CommandOutput::structured(json!({})))
        });
    }
    registry
}

/// RFC 0017's owner-local adoption of the released per-operation input
/// schemas. The observation supplies the frozen property contracts; Twill
/// owns their property-level authoring and the one catalog presence relation
/// that v0.4.9 enforced outside its ungrouped schemas.
pub fn argument_schema_registry(baseline: &Value) -> CommandRegistry {
    let paths = OPERATION_MAPPING
        .iter()
        .map(|(released, path, title)| (*released, (*path, *title)))
        .collect::<std::collections::BTreeMap<_, _>>();
    let mut registry = CommandRegistry::new(
        "vbl-argument-schemas",
        "Visible Browser Lab argument schema adoption fixture",
    );
    for tool in baseline.as_array().expect("VBL baseline tools") {
        let name = tool["name"].as_str().expect("VBL tool name");
        let (path, title) = paths[name];
        let input = tool["inputSchema"].as_object().expect("VBL input schema");
        let required = input["required"]
            .as_array()
            .expect("VBL required list")
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>();
        let mut spec = CommandSpec::new(
            path.split_whitespace(),
            title,
            format!("RFC 0017 VBL schema adoption for `{name}`."),
        );
        let properties = input["properties"]
            .as_object()
            .expect("VBL property schemas");
        let argument_order = required
            .iter()
            .copied()
            .chain(
                properties
                    .keys()
                    .map(String::as_str)
                    .filter(|argument| !required.contains(argument)),
            )
            .collect::<Vec<_>>();
        for argument in argument_order {
            let schema = &properties[argument];
            let summary = schema
                .get("description")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
                .unwrap_or_else(|| format!("VBL `{argument}` argument"));
            let mut arg = ArgSpec::inline_schema(argument, schema.clone(), summary);
            if !required.contains(&argument) {
                arg = arg.optional();
            }
            if name == "screencast_start" && argument == "max_width" {
                arg = arg.requires_argument("max_height");
            }
            if name == "screencast_start" && argument == "max_height" {
                arg = arg.requires_argument("max_width");
            }
            spec = spec.with_arg(arg);
        }
        registry = registry.register(spec, |_context| async {
            Ok(CommandOutput::structured(json!({})))
        });
    }
    registry
}
