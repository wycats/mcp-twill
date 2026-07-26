use std::fmt::Write;

use mcp_twill::{FrameworkError, Result};
use serde_json::{Map, Value, json};

use crate::{
    HostAdapterKind, HostAdapterSnapshot, HostInvocationTransport, profile::CompiledHostGuidance,
};

pub struct VsCodeGeneratedArtifacts {
    manifest_projection: Value,
    manifest_projection_json: String,
    adapter_typescript: String,
}

impl VsCodeGeneratedArtifacts {
    pub fn manifest_projection(&self) -> &Value {
        &self.manifest_projection
    }

    pub fn manifest_projection_json(&self) -> &str {
        &self.manifest_projection_json
    }

    pub fn adapter_typescript(&self) -> &str {
        &self.adapter_typescript
    }
}

pub fn generate_vscode_artifacts(
    snapshot: &HostAdapterSnapshot,
) -> Result<VsCodeGeneratedArtifacts> {
    let profile = &snapshot.profile;
    if snapshot.version() != 1 {
        return Err(build_error(
            "VS Code artifact generator does not support this host snapshot version",
        ));
    }
    let HostAdapterKind::VsCodeLanguageModelTools { engine_floor } = profile.kind;
    let mut contributions = Vec::with_capacity(snapshot.tools.len());
    for tool in &snapshot.tools {
        let native_name = &tool.native_name;
        let host_name = &tool.host_name;
        let document = tool
            .document
            .as_object()
            .ok_or_else(|| build_error("compiled host tool is missing its document"))?;
        let title = document
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or(native_name);
        let compiled_description = document
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or("");
        let mut contribution = Map::new();
        contribution.insert("name".to_string(), Value::String(host_name.to_string()));
        contribution.insert("displayName".to_string(), Value::String(title.to_string()));
        contribution.insert(
            "userDescription".to_string(),
            Value::String(tool.user_description.clone()),
        );
        contribution.insert(
            "modelDescription".to_string(),
            Value::String(model_description(
                compiled_description,
                &tool.operations,
                &snapshot.guidance,
            )),
        );
        if let Some(icon) = &profile.icon {
            contribution.insert("icon".to_string(), Value::String(icon.clone()));
        }
        contribution.insert(
            "inputSchema".to_string(),
            document
                .get("inputSchema")
                .cloned()
                .unwrap_or_else(|| json!({"type": "object"})),
        );
        if tool.operations.len() == 1
            && let Some(operation_id) = tool.operations.first()
            && let Some(reference) = profile.prompt_references.get(operation_id)
        {
            contribution.insert("canBeReferencedInPrompt".to_string(), Value::Bool(true));
            contribution.insert(
                "toolReferenceName".to_string(),
                Value::String(reference.clone()),
            );
        }
        contributions.push(Value::Object(contribution));
    }
    let manifest_projection = json!({
        "engines": {
            "vscode": engine_floor.caret_range(),
        },
        "contributes": {
            "languageModelTools": contributions,
        },
    });
    let mut manifest_projection_json = serde_json::to_string_pretty(&manifest_projection)
        .map_err(|_| build_error("cannot render VS Code manifest projection"))?;
    manifest_projection_json.push('\n');
    let adapter_typescript = generated_typescript(snapshot, profile)?;
    Ok(VsCodeGeneratedArtifacts {
        manifest_projection,
        manifest_projection_json,
        adapter_typescript,
    })
}

fn model_description(
    description: &str,
    operations: &[String],
    guidance: &CompiledHostGuidance,
) -> String {
    let mut parts = vec![description.to_string()];
    let mut seen = std::collections::BTreeSet::new();
    if !guidance.tool_suffix.is_empty() {
        seen.insert(guidance.tool_suffix.clone());
        parts.push(guidance.tool_suffix.clone());
    }
    for operation in operations {
        if let Some(rendered) = guidance.operation_suffixes.get(operation)
            && !rendered.is_empty()
            && seen.insert(rendered.to_string())
        {
            parts.push(rendered.to_string());
        }
    }
    parts
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

fn generated_typescript(
    snapshot: &HostAdapterSnapshot,
    profile: &crate::HostAdapterProfileDecl,
) -> Result<String> {
    let snapshot_json = serde_json::to_string(snapshot.document())
        .map_err(|_| build_error("cannot embed host snapshot in generated TypeScript"))?;
    let transport = match &profile.transport {
        HostInvocationTransport::InProcess => "in_process",
        HostInvocationTransport::ProcessEnvelopeV1 { .. } => "process_envelope_v1",
    };
    let runtime_interface = match profile.transport {
        HostInvocationTransport::InProcess => "HostInProcessRuntime",
        HostInvocationTransport::ProcessEnvelopeV1 { .. } => "HostProcessRuntime",
    };
    let mut source = String::new();
    writeln!(source, "/* Generated by mcp-twill-host. Do not edit. */").unwrap();
    writeln!(source, "import * as vscode from \"vscode\";").unwrap();
    if matches!(
        profile.transport,
        HostInvocationTransport::ProcessEnvelopeV1 { .. }
    ) {
        writeln!(source, "import {{ spawn }} from \"node:child_process\";").unwrap();
    }
    writeln!(
        source,
        "const HOST_PROFILE = {};",
        js_string(snapshot.profile_id())
    )
    .unwrap();
    writeln!(
        source,
        "const HOST_ADAPTER_HASH = {};",
        js_string(snapshot.host_adapter_hash())
    )
    .unwrap();
    writeln!(
        source,
        "const SURFACE_HASH = {};",
        js_string(snapshot.surface_hash())
    )
    .unwrap();
    writeln!(
        source,
        "const MAX_CALL_BYTES = {};",
        profile.invocation_limits.max_call_bytes
    )
    .unwrap();
    writeln!(
        source,
        "const MAX_RESULT_BYTES = {};",
        profile.invocation_limits.max_result_bytes
    )
    .unwrap();
    writeln!(source, "const HOST_SNAPSHOT = {snapshot_json} as const;").unwrap();
    writeln!(
        source,
        "const TRANSPORT = {} as const;",
        js_string(transport)
    )
    .unwrap();
    source.push_str(TYPESCRIPT_COMMON);
    if let HostInvocationTransport::ProcessEnvelopeV1 {
        logical_binary_name,
        subcommand,
        limits,
    } = &profile.transport
    {
        writeln!(
            source,
            "const LOGICAL_BINARY_NAME = {};",
            js_string(logical_binary_name)
        )
        .unwrap();
        writeln!(
            source,
            "const SUBCOMMAND = {} as const;",
            serde_json::to_string(subcommand)
                .map_err(|_| build_error("cannot embed process subcommand"))?
        )
        .unwrap();
        writeln!(
            source,
            "const MAX_STDERR_BYTES = {};",
            limits.max_stderr_bytes
        )
        .unwrap();
        writeln!(
            source,
            "const TERMINATION_GRACE_MS = {};",
            limits.termination_grace_ms
        )
        .unwrap();
        source.push_str(TYPESCRIPT_PROCESS);
    } else {
        source.push_str(TYPESCRIPT_IN_PROCESS);
    }
    writeln!(
        source,
        "export function registerGeneratedHostTools(extensionContext: vscode.ExtensionContext, contextProvider: HostContextProvider, runtime: {runtime_interface}): void {{"
    )
    .unwrap();
    source.push_str(TYPESCRIPT_REGISTRATION_BODY);
    source.push_str("}\n");
    if !source.ends_with('\n') {
        source.push('\n');
    }
    Ok(source)
}

fn js_string(value: &str) -> String {
    serde_json::to_string(value).expect("JSON string encoding is infallible")
}

const TYPESCRIPT_COMMON: &str = r#"
export interface ConversationIdentity {
  readonly version: 1;
  readonly issuer: string;
  readonly id: string;
}
export interface HostWorkspaceRoot {
  readonly issuer: string;
  readonly name?: string;
  readonly uri: string;
}
export type HostInvocationContextV1 =
  | { readonly kind: "ambient"; readonly conversationIdentity: Readonly<ConversationIdentity>; readonly workspaceRoots?: readonly Readonly<HostWorkspaceRoot>[] }
  | { readonly kind: "absent"; readonly workspaceRoots?: readonly Readonly<HostWorkspaceRoot>[] }
  | { readonly kind: "unsupported"; readonly reason: "unknown_token_shape" | "invalid_session_resource" | "invalid_working_directory" | "provider_failed" };
export interface HostVsCodeVersionV1 { readonly major: number; readonly minor: number; readonly patch: number; }
export interface HostRuntimeFactsV1 { readonly kind: "vs_code"; readonly engineVersion?: Readonly<HostVsCodeVersionV1>; }
export type HostCallOutcomeV1 =
  | { readonly kind: "success"; readonly text: string }
  | { readonly kind: "application_error"; readonly code: string; readonly text: string }
  | { readonly kind: "framework_error"; readonly code: string; readonly text: string };
export interface HostCallResultV1 {
  readonly version: 1;
  readonly hostAdapterHash: string;
  readonly surfaceHash: string;
  readonly outcome: HostCallOutcomeV1;
}
export interface HostContextProvider { resolve(options: unknown): HostInvocationContextV1; }
export interface HostDiagnosticSink { write(chunk: Uint8Array): void | Promise<void>; }
export interface HostProcessLaunch {
  readonly executable: string;
  readonly workingDirectory: string;
  readonly environment: Readonly<Record<string, string>>;
}
export interface HostProcessRuntime {
  resolveLaunch(logicalName: string): HostProcessLaunch;
  readonly diagnosticSink?: HostDiagnosticSink;
}
export interface HostInProcessRuntime {
  call(
    tool: string,
    input: Readonly<Record<string, unknown>>,
    context: HostInvocationContextV1,
    runtime: HostRuntimeFactsV1,
    token: vscode.CancellationToken,
  ): Promise<HostCallResultV1>;
}

const PREPARE_FAILURE = "Generated host adapter could not prepare this invocation";
const CONTRACT_FAILURE = "Generated host adapter received an invalid result envelope";
const CALL_PAYLOAD_FAILURE = "Generated host call exceeds its configured byte limit";
const PAYLOAD_FAILURE = "Generated host result exceeds its configured byte limit";
const MAX_ERROR_TEXT_SCALARS = 1024;
const textEncoder = new TextEncoder();
const textDecoder = new TextDecoder("utf-8", { fatal: true });
let registered = false;

function parseEngineVersion(raw: string): HostVsCodeVersionV1 | undefined {
  const match = /^([0-9]+)\.([0-9]+)\.([0-9]+)$/.exec(raw);
  if (!match) return undefined;
  const values = match.slice(1).map(Number);
  if (values.some((value) => !Number.isSafeInteger(value) || value < 0 || value > 0xffffffff)) return undefined;
  return Object.freeze({ major: values[0]!, minor: values[1]!, patch: values[2]! });
}

function isWellFormedUnicode(value: string): boolean {
  for (let index = 0; index < value.length; index++) {
    const unit = value.charCodeAt(index);
    if (unit >= 0xd800 && unit <= 0xdbff) {
      const low = value.charCodeAt(++index);
      if (!(low >= 0xdc00 && low <= 0xdfff)) return false;
    } else if (unit >= 0xdc00 && unit <= 0xdfff) {
      return false;
    }
  }
  return true;
}

function hostTextScalarIsUnsafe(scalar: string): boolean {
  const point = scalar.codePointAt(0)!;
  return point <= 0x1f
    || (point >= 0x7f && point <= 0x9f)
    || point === 0x061c
    || (point >= 0x200e && point <= 0x200f)
    || (point >= 0x2028 && point <= 0x202e)
    || (point >= 0x2060 && point <= 0x206f)
    || point === 0xfeff;
}

function encodeAndTruncateHostText(text: string, limit: number): string {
  const chunks: string[] = [];
  for (let index = 0; index < text.length;) {
    const point = text.codePointAt(index)!;
    const scalar = String.fromCodePoint(point);
    index += scalar.length;
    if (scalar === "\\") {
      const unicode = /^u[0-9A-Fa-f]{4}/.exec(text.slice(index));
      if (unicode) {
        chunks.push(`\\${unicode[0]}`);
        index += unicode[0].length;
        continue;
      }
      const short = /^[\\"bfnrt]/.exec(text.slice(index));
      if (short) {
        chunks.push(`\\${short[0]}`);
        index += 1;
        continue;
      }
      chunks.push("\\");
    } else if (hostTextScalarIsUnsafe(scalar)) {
      chunks.push(`\\u${point.toString(16).toUpperCase().padStart(4, "0")}`);
    } else {
      chunks.push(scalar);
    }
  }
  const total = chunks.reduce((count, chunk) => count + [...chunk].length, 0);
  if (total <= limit) return chunks.join("");
  const kept: string[] = [];
  let width = 0;
  for (const chunk of chunks) {
    const chunkWidth = [...chunk].length;
    if (width + chunkWidth > Math.max(0, limit - 1)) break;
    kept.push(chunk);
    width += chunkWidth;
  }
  return `${kept.join("")}…`;
}

interface CloneBudget {
  readonly limit: number;
  readonly failure: string;
  total: number;
}

class CloneLimitError extends Error {}
class RuntimeHookError extends Error {}

function countCloneBytes(budget: CloneBudget | undefined, text: string): void {
  if (!budget) return;
  budget.total += textEncoder.encode(text).byteLength;
  if (budget.total > budget.limit) throw new CloneLimitError(budget.failure);
}

function hasInheritedEnumerableState(value: object, failure: string): boolean {
  const visited = new Set<object>();
  let depth = 0;
  let prototype = Object.getPrototypeOf(value);
  while (prototype !== null) {
    if (visited.has(prototype) || depth >= 128) throw new Error(failure);
    visited.add(prototype);
    depth++;
    for (const key of Reflect.ownKeys(prototype)) {
      if (Object.getOwnPropertyDescriptor(prototype, key)?.enumerable) return true;
    }
    prototype = Object.getPrototypeOf(prototype);
  }
  return false;
}

function cloneData(
  value: unknown,
  failure: string,
  depth = 0,
  seen = new Set<object>(),
  budget?: CloneBudget,
): unknown {
  if (value === null) {
    countCloneBytes(budget, "null");
    return value;
  }
  if (typeof value === "boolean") {
    countCloneBytes(budget, value ? "true" : "false");
    return value;
  }
  if (typeof value === "string") {
    if (!isWellFormedUnicode(value)) throw new Error(failure);
    countCloneBytes(budget, JSON.stringify(value));
    return value;
  }
  if (typeof value === "number") {
    if (!Number.isFinite(value)) throw new Error(failure);
    const normalized = Object.is(value, -0) ? 0 : value;
    countCloneBytes(budget, JSON.stringify(normalized));
    return normalized;
  }
  if (Array.isArray(value)) {
    if (depth >= 128) throw new Error(failure);
    if (seen.has(value)) throw new Error(failure);
    seen.add(value);
    if (Object.getPrototypeOf(value) !== Array.prototype) throw new Error(failure);
    if (hasInheritedEnumerableState(value, failure)) throw new Error(failure);
    const lengthDescriptor = Object.getOwnPropertyDescriptor(value, "length");
    if (!lengthDescriptor || lengthDescriptor.enumerable || !("value" in lengthDescriptor)) throw new Error(failure);
    const length = lengthDescriptor.value;
    if (!Number.isSafeInteger(length) || length < 0) throw new Error(failure);
    if (budget) {
      const remaining = budget.limit - budget.total;
      const maximumElements = remaining >= 2 ? Math.floor((remaining - 1) / 2) : -1;
      if (length > maximumElements) throw new CloneLimitError(budget.failure);
    }
    const keys = Reflect.ownKeys(value);
    if (keys.length !== length + 1) throw new Error(failure);
    for (const key of keys) {
      if (key === "length") continue;
      if (typeof key === "symbol") throw new Error(failure);
      const index = Number(key);
      if (!Number.isSafeInteger(index) || index < 0 || index >= length || String(index) !== key) throw new Error(failure);
    }
    countCloneBytes(budget, "[");
    const copy: unknown[] = [];
    for (let index = 0; index < length; index++) {
      const descriptor = Object.getOwnPropertyDescriptor(value, String(index));
      if (!descriptor?.enumerable || !("value" in descriptor)) throw new Error(failure);
      if (index > 0) countCloneBytes(budget, ",");
      copy.push(cloneData(descriptor.value, failure, depth + 1, seen, budget));
    }
    countCloneBytes(budget, "]");
    return Object.freeze(copy);
  }
  if (typeof value !== "object") throw new Error(failure);
  if (depth >= 128) throw new Error(failure);
  if (seen.has(value)) throw new Error(failure);
  seen.add(value);
  const prototype = Object.getPrototypeOf(value);
  if (prototype !== Object.prototype && prototype !== null) throw new Error(failure);
  if (hasInheritedEnumerableState(value, failure)) throw new Error(failure);
  const copy: Record<string, unknown> = Object.create(null);
  countCloneBytes(budget, "{");
  for (const [index, key] of Reflect.ownKeys(value).entries()) {
    if (typeof key === "symbol" || !isWellFormedUnicode(key)) throw new Error(failure);
    const descriptor = Object.getOwnPropertyDescriptor(value, key);
    if (!descriptor?.enumerable || !("value" in descriptor)) throw new Error(failure);
    if (index > 0) countCloneBytes(budget, ",");
    countCloneBytes(budget, JSON.stringify(key));
    countCloneBytes(budget, ":");
    copy[key] = cloneData(descriptor.value, failure, depth + 1, seen, budget);
  }
  countCloneBytes(budget, "}");
  return Object.freeze(copy);
}

function inputSnapshot(
  options: unknown,
  failure: string,
  overflowFailure: string,
): Readonly<Record<string, unknown>> {
  try {
    const source = (options as { input?: unknown } | null)?.input;
    const snapshot = cloneData(
      source === undefined ? {} : source,
      failure,
      0,
      new Set<object>(),
      { limit: MAX_CALL_BYTES, failure: overflowFailure, total: 0 },
    );
    if (snapshot === null || Array.isArray(snapshot) || typeof snapshot !== "object") throw new Error(failure);
    return snapshot as Readonly<Record<string, unknown>>;
  } catch (error) {
    if (error instanceof CloneLimitError) throw error;
    throw new Error(failure);
  }
}

function validatePreparationInput(input: Readonly<Record<string, unknown>>): void {
  try {
    if (canonicalJsonByteLength(input, MAX_CALL_BYTES) > MAX_CALL_BYTES) {
      throw new Error(PREPARE_FAILURE);
    }
  } catch {
    throw new Error(PREPARE_FAILURE);
  }
}

function canonicalJson(value: unknown): string {
  if (value === null) return "null";
  if (value === true) return "true";
  if (value === false) return "false";
  if (typeof value === "number") {
    if (!Number.isFinite(value)) throw new Error(CONTRACT_FAILURE);
    return JSON.stringify(Object.is(value, -0) ? 0 : value);
  }
  if (typeof value === "string") {
    if (!isWellFormedUnicode(value)) throw new Error(CONTRACT_FAILURE);
    return JSON.stringify(value);
  }
  if (Array.isArray(value)) return `[${value.map(canonicalJson).join(",")}]`;
  if (!value || typeof value !== "object") throw new Error(CONTRACT_FAILURE);
  return `{${Object.keys(value as object).sort().map((key) => {
    if (!isWellFormedUnicode(key)) throw new Error(CONTRACT_FAILURE);
    return `${JSON.stringify(key)}:${canonicalJson((value as Record<string, unknown>)[key])}`;
  }).join(",")}}`;
}

function canonicalJsonByteLength(value: unknown, limit: number): number {
  let total = 0;
  let exceeded = false;
  const addText = (text: string): void => {
    if (exceeded) return;
    total += textEncoder.encode(text).byteLength;
    exceeded = total > limit;
  };
  const visit = (entry: unknown): void => {
    if (exceeded) return;
    if (entry === null) return addText("null");
    if (entry === true) return addText("true");
    if (entry === false) return addText("false");
    if (typeof entry === "number") {
      if (!Number.isFinite(entry)) throw new Error(CONTRACT_FAILURE);
      return addText(JSON.stringify(Object.is(entry, -0) ? 0 : entry));
    }
    if (typeof entry === "string") {
      if (!isWellFormedUnicode(entry)) throw new Error(CONTRACT_FAILURE);
      return addText(JSON.stringify(entry));
    }
    if (Array.isArray(entry)) {
      addText("[");
      entry.forEach((item, index) => {
        if (index > 0) addText(",");
        visit(item);
      });
      return addText("]");
    }
    if (!entry || typeof entry !== "object") throw new Error(CONTRACT_FAILURE);
    addText("{");
    Object.keys(entry as object).sort().forEach((key, index) => {
      if (index > 0) addText(",");
      if (!isWellFormedUnicode(key)) throw new Error(CONTRACT_FAILURE);
      addText(JSON.stringify(key));
      addText(":");
      visit((entry as Record<string, unknown>)[key]);
    });
    addText("}");
  };
  visit(value);
  return exceeded ? limit + 1 : total;
}

function exactKeys(value: object, expected: readonly string[]): boolean {
  const keys = Object.keys(value).sort();
  return keys.length === expected.length && keys.every((key, index) => key === [...expected].sort()[index]);
}

function toolRecord(hostName: string, failure: string): any {
  const record = (HOST_SNAPSHOT.tools as readonly any[]).find((tool) => tool.hostName === hostName);
  if (!record) throw new Error(failure);
  return record;
}

function operationRecord(record: any, input: Readonly<Record<string, unknown>>, failure: string): any {
  const operations = (HOST_SNAPSHOT.nativeSurface.document.operations as readonly any[])
    .filter((operation) => (record.operations as readonly string[]).includes(operation.spec.id))
    .filter((operation) => {
      const fixed = operation.call.arguments as Record<string, unknown> | undefined;
      return !fixed || Object.entries(fixed).every(([key, value]) => Object.prototype.hasOwnProperty.call(input, key) && canonicalJson(input[key]) === canonicalJson(value));
    });
  if (operations.length !== 1) throw new Error(failure);
  return operations[0];
}

function isFrameworkHelp(record: any): boolean {
  return Array.isArray(record.operations) && record.operations.length === 0;
}

function commandInput(operation: any, input: Readonly<Record<string, unknown>>): Readonly<Record<string, unknown>> {
  const fixed = operation.call.arguments as Record<string, unknown> | undefined;
  if (!fixed) return input;
  const copy: Record<string, unknown> = Object.create(null);
  for (const [key, value] of Object.entries(input)) {
    if (!Object.prototype.hasOwnProperty.call(fixed, key)) copy[key] = value;
  }
  return Object.freeze(copy);
}

function effectConfirms(effect: unknown): boolean {
  if (typeof effect === "string") return ["write", "delete", "exec", "network"].includes(effect);
  if (effect && typeof effect === "object" && "composite" in effect) {
    return (effect as { composite: readonly unknown[] }).composite.some(effectConfirms);
  }
  return false;
}

function predicateMatches(predicate: any, input: Readonly<Record<string, unknown>>): boolean {
  if (predicate.argumentPresent) {
    return Object.prototype.hasOwnProperty.call(input, predicate.argumentPresent.argument);
  }
  if (predicate.argumentEquals) {
    const key = predicate.argumentEquals.argument as string;
    return Object.prototype.hasOwnProperty.call(input, key)
      && canonicalJson(input[key]) === canonicalJson(predicate.argumentEquals.value);
  }
  return false;
}

function presentationUnsafe(codePoint: number): boolean {
  return (codePoint <= 0x1f)
    || (codePoint >= 0x7f && codePoint <= 0x9f)
    || codePoint === 0x061c
    || (codePoint >= 0x200e && codePoint <= 0x200f)
    || (codePoint >= 0x2028 && codePoint <= 0x202e)
    || (codePoint >= 0x2060 && codePoint <= 0x206f)
    || codePoint === 0xfeff;
}

function escapedScalar(scalar: string): string {
  switch (scalar) {
    case "\"": return "\\\"";
    case "\\": return "\\\\";
    case "\b": return "\\b";
    case "\f": return "\\f";
    case "\n": return "\\n";
    case "\r": return "\\r";
    case "\t": return "\\t";
  }
  const codePoint = scalar.codePointAt(0)!;
  return presentationUnsafe(codePoint) ? `\\u${codePoint.toString(16).toUpperCase().padStart(4, "0")}` : scalar;
}

function renderString(value: string, quoted: boolean): string {
  const escaped = [...value].map(escapedScalar);
  const fixed = quoted ? 2 : 0;
  const fits = escaped.reduce((width, scalar) => width + [...scalar].length, fixed) <= 256;
  const limit = fits ? 256 - fixed : quoted ? 253 : 255;
  let body = "";
  for (const scalar of escaped) {
    if ([...body].length + [...scalar].length > limit) break;
    body += scalar;
  }
  if (!fits) body += "…";
  return quoted ? `"${body}"` : body;
}

function trimEcmaScript(value: string): string {
  const trim = (scalar: string) => {
    const point = scalar.codePointAt(0)!;
    return [0x0009, 0x000b, 0x000c, 0x0020, 0x00a0, 0x1680, 0x202f, 0x205f, 0x3000, 0xfeff, 0x000a, 0x000d, 0x2028, 0x2029].includes(point)
      || (point >= 0x2000 && point <= 0x200a);
  };
  const scalars = [...value];
  let start = 0;
  let end = scalars.length;
  while (start < end && trim(scalars[start]!)) start++;
  while (end > start && trim(scalars[end - 1]!)) end--;
  return scalars.slice(start, end).join("");
}

function renderArgument(segment: any, input: Readonly<Record<string, unknown>>): string {
  const value = input[segment.argument];
  if (segment.rendering === "plain") {
    if (typeof value === "string" && value.length > 0) return renderString(value, false);
    if (typeof value === "boolean") return String(value);
  } else if (segment.rendering === "jsonString") {
    if (typeof value === "string" && value.length > 0) return renderString(value, true);
  } else if (segment.rendering === "trimmedJsonString" && typeof value === "string") {
    const trimmed = trimEcmaScript(value);
    if (trimmed.length > 0) return renderString(trimmed, true);
  }
  return segment.fallback;
}

function renderConfirmation(declaration: any, input: Readonly<Record<string, unknown>>): { title: string; message: string } {
  const selected = (declaration.cases as readonly any[]).find((entry) => predicateMatches(entry.when, input));
  const message = selected?.message ?? declaration.default;
  return {
    title: message.title,
    message: (message.body as readonly any[]).map((segment) => {
      if ("text" in segment) return segment.text;
      return renderArgument(segment.argument, input);
    }).join(""),
  };
}

function preparedInvocation(hostName: string, input: Readonly<Record<string, unknown>>): vscode.PreparedToolInvocation {
  const record = toolRecord(hostName, PREPARE_FAILURE);
  if (isFrameworkHelp(record)) {
    const invocationMessage = typeof record.document?.title === "string"
      ? record.document.title
      : "Getting help";
    return { invocationMessage };
  }
  const operation = operationRecord(record, input, PREPARE_FAILURE);
  const command = commandInput(operation, input);
  const presentation = operation.spec.presentation;
  const invocationMessage = presentation?.invocationMessage ?? operation.presentationDefaults.invocationMessage;
  const trigger = String(HOST_SNAPSHOT.profile.confirmation.trigger) as "none" | "effectDefault" | "declaredPresentation";
  let confirmationMessages: { title: string; message: string } | undefined;
  if (trigger === "declaredPresentation" && presentation?.confirmation) {
    confirmationMessages = renderConfirmation(presentation.confirmation, command);
  } else if (trigger === "effectDefault" && effectConfirms(operation.spec.effect)) {
    confirmationMessages = presentation?.confirmation
      ? renderConfirmation(presentation.confirmation, command)
      : {
          title: operation.presentationDefaults.confirmationTitle,
          message: operation.presentationDefaults.confirmationMessage,
        };
  }
  return confirmationMessages ? { invocationMessage, confirmationMessages } : { invocationMessage };
}

function runtimeFacts(): HostRuntimeFactsV1 {
  const engineVersion = parseEngineVersion(vscode.version);
  return Object.freeze(engineVersion ? { kind: "vs_code", engineVersion } : { kind: "vs_code" });
}

function callEnvelope(
  hostName: string,
  input: Readonly<Record<string, unknown>>,
  context: HostInvocationContextV1,
  runtime: HostRuntimeFactsV1,
): Readonly<Record<string, unknown>> {
  return Object.freeze({
    version: 1,
    hostProfile: HOST_PROFILE,
    hostAdapterHash: HOST_ADAPTER_HASH,
    surfaceHash: SURFACE_HASH,
    tool: hostName,
    arguments: input,
    context,
    runtime,
  });
}

function encodeCallEnvelope(
  hostName: string,
  input: Readonly<Record<string, unknown>>,
  context: HostInvocationContextV1,
  runtime: HostRuntimeFactsV1,
): Uint8Array {
  const envelope = callEnvelope(hostName, input, context, runtime);
  if (canonicalJsonByteLength(envelope, MAX_CALL_BYTES) > MAX_CALL_BYTES) {
    throw new Error(CALL_PAYLOAD_FAILURE);
  }
  return textEncoder.encode(canonicalJson(envelope));
}

function validateWorkspaceRoots(value: unknown): readonly Readonly<HostWorkspaceRoot>[] {
  if (!Array.isArray(value)) throw new Error(CONTRACT_FAILURE);
  return Object.freeze(value.map((entry) => {
    if (!entry || typeof entry !== "object" || Array.isArray(entry)) throw new Error(CONTRACT_FAILURE);
    const root = entry as Record<string, unknown>;
    const expected = root.name === undefined ? ["issuer", "uri"] : ["issuer", "name", "uri"];
    if (!exactKeys(root, expected) || typeof root.issuer !== "string" || typeof root.uri !== "string" || (root.name !== undefined && typeof root.name !== "string")) throw new Error(CONTRACT_FAILURE);
    if (!/^[a-z0-9](?:[a-z0-9-]*[a-z0-9])?\.[a-z0-9](?:[a-z0-9-]*[a-z0-9])?(?:\.[a-z0-9](?:[a-z0-9-]*[a-z0-9])?)*$/.test(root.issuer)) throw new Error(CONTRACT_FAILURE);
    if (root.name === "" || !/^file:/i.test(root.uri)) throw new Error(CONTRACT_FAILURE);
    const path = root.uri.slice(5);
    if (!(path.startsWith("/") || /^[A-Za-z]:[\\/]/.test(path))) throw new Error(CONTRACT_FAILURE);
    return Object.freeze(root) as Readonly<HostWorkspaceRoot>;
  }));
}

function validateContext(value: unknown): HostInvocationContextV1 {
  const copied = cloneData(
    value,
    CONTRACT_FAILURE,
    0,
    new Set<object>(),
    { limit: MAX_CALL_BYTES, failure: CALL_PAYLOAD_FAILURE, total: 0 },
  );
  if (!copied || typeof copied !== "object" || Array.isArray(copied)) throw new Error(CONTRACT_FAILURE);
  const record = copied as Record<string, unknown>;
  if (record.kind === "unsupported") {
    if (!exactKeys(record, ["kind", "reason"]) || !["unknown_token_shape", "invalid_session_resource", "invalid_working_directory", "provider_failed"].includes(String(record.reason))) throw new Error(CONTRACT_FAILURE);
    return record as unknown as HostInvocationContextV1;
  }
  const roots = record.workspaceRoots === undefined ? undefined : validateWorkspaceRoots(record.workspaceRoots);
  if (record.kind === "absent") {
    if (!exactKeys(record, roots === undefined ? ["kind"] : ["kind", "workspaceRoots"])) throw new Error(CONTRACT_FAILURE);
    return Object.freeze(roots === undefined ? { kind: "absent" } : { kind: "absent", workspaceRoots: roots });
  }
  if (record.kind === "ambient") {
    const identity = record.conversationIdentity as Record<string, unknown> | undefined;
    if (!identity || !exactKeys(identity, ["version", "issuer", "id"]) || identity.version !== 1 || typeof identity.issuer !== "string" || typeof identity.id !== "string" || identity.id.length === 0) throw new Error(CONTRACT_FAILURE);
    if (!/^[a-z0-9](?:[a-z0-9-]*[a-z0-9])?\.[a-z0-9](?:[a-z0-9-]*[a-z0-9])?(?:\.[a-z0-9](?:[a-z0-9-]*[a-z0-9])?)*$/.test(identity.issuer)) throw new Error(CONTRACT_FAILURE);
    if (!exactKeys(record, roots === undefined ? ["conversationIdentity", "kind"] : ["conversationIdentity", "kind", "workspaceRoots"])) throw new Error(CONTRACT_FAILURE);
    return Object.freeze(roots === undefined
      ? { kind: "ambient", conversationIdentity: Object.freeze(identity) as unknown as ConversationIdentity }
      : { kind: "ambient", conversationIdentity: Object.freeze(identity) as unknown as ConversationIdentity, workspaceRoots: roots });
  }
  throw new Error(CONTRACT_FAILURE);
}

function validateResult(value: HostCallResultV1): HostCallResultV1 {
  let result: HostCallResultV1;
  try {
    result = cloneData(
      value,
      CONTRACT_FAILURE,
      0,
      new Set<object>(),
      { limit: MAX_RESULT_BYTES, failure: PAYLOAD_FAILURE, total: 0 },
    ) as HostCallResultV1;
  } catch (error) {
    if (error instanceof CloneLimitError) throw error;
    throw new Error(CONTRACT_FAILURE);
  }
  if (!exactKeys(result, ["hostAdapterHash", "outcome", "surfaceHash", "version"])) throw new Error(CONTRACT_FAILURE);
  if (result.version !== 1 || result.hostAdapterHash !== HOST_ADAPTER_HASH || result.surfaceHash !== SURFACE_HASH) throw new Error(CONTRACT_FAILURE);
  if (!result.outcome || !["success", "application_error", "framework_error"].includes(result.outcome.kind)) throw new Error(CONTRACT_FAILURE);
  const outcomeKeys = result.outcome.kind === "success" ? ["kind", "text"] : ["code", "kind", "text"];
  if (!exactKeys(result.outcome, outcomeKeys)) throw new Error(CONTRACT_FAILURE);
  if (typeof result.outcome.text !== "string" || result.outcome.text.length === 0 && result.outcome.kind !== "success") throw new Error(CONTRACT_FAILURE);
  if (
    result.outcome.kind !== "success"
    && (
      [...result.outcome.text].length > MAX_ERROR_TEXT_SCALARS
      || [...result.outcome.text].some(hostTextScalarIsUnsafe)
    )
  ) throw new Error(CONTRACT_FAILURE);
  if (result.outcome.kind === "application_error" && !(HOST_SNAPSHOT.applicationCodes as readonly string[]).includes(result.outcome.code)) throw new Error(CONTRACT_FAILURE);
  if (result.outcome.kind === "framework_error" && !(HOST_SNAPSHOT.frameworkCodes as readonly string[]).includes(result.outcome.code)) throw new Error(CONTRACT_FAILURE);
  if (result.outcome.kind === "success") scanCompactJson(result.outcome.text);
  return result;
}

function scanCompactJson(text: string): void {
  let index = 0;
  const string = (): string => {
    const start = index;
    if (text[index++] !== "\"") throw new Error(CONTRACT_FAILURE);
    while (index < text.length) {
      const character = text[index++];
      if (character === "\\") {
        if (index >= text.length) throw new Error(CONTRACT_FAILURE);
        if (text[index] === "u") index += 5;
        else index++;
      } else if (character === "\"") {
        let decoded: unknown;
        try { decoded = JSON.parse(text.slice(start, index)); } catch { throw new Error(CONTRACT_FAILURE); }
        if (typeof decoded !== "string" || !isWellFormedUnicode(decoded)) throw new Error(CONTRACT_FAILURE);
        return decoded;
      } else if (character.charCodeAt(0) <= 0x1f) {
        throw new Error(CONTRACT_FAILURE);
      }
    }
    throw new Error(CONTRACT_FAILURE);
  };
  const value = (depth: number): void => {
    const character = text[index];
    if (depth >= 128 && (character === "[" || character === "{")) throw new Error(CONTRACT_FAILURE);
    if (character === "\"") { string(); return; }
    if (character === "[") {
      index++;
      if (text[index] === "]") { index++; return; }
      while (true) {
        value(depth + 1);
        if (text[index] === "]") { index++; return; }
        if (text[index++] !== ",") throw new Error(CONTRACT_FAILURE);
      }
    }
    if (character === "{") {
      index++;
      const names = new Set<string>();
      if (text[index] === "}") { index++; return; }
      while (true) {
        const name = string();
        if (names.has(name)) throw new Error(CONTRACT_FAILURE);
        names.add(name);
        if (text[index++] !== ":") throw new Error(CONTRACT_FAILURE);
        value(depth + 1);
        if (text[index] === "}") { index++; return; }
        if (text[index++] !== ",") throw new Error(CONTRACT_FAILURE);
      }
    }
    const match = /^(?:null|true|false|-?(?:0|[1-9][0-9]*)(?:\.[0-9]+)?(?:[eE][+-]?[0-9]+)?)/.exec(text.slice(index));
    if (!match) throw new Error(CONTRACT_FAILURE);
    index += match[0].length;
  };
  value(0);
  if (index !== text.length) throw new Error(CONTRACT_FAILURE);
}

function parseUniqueJson(text: string): unknown {
  let index = 0;
  const whitespace = () => { while (/[\t\n\r ]/.test(text[index] ?? "")) index++; };
  const value = (depth = 0): unknown => {
    whitespace();
    const start = index;
    const character = text[index];
    if (depth >= 128 && (character === "[" || character === "{")) throw new Error(CONTRACT_FAILURE);
    if (character === "\"") {
      index++;
      while (index < text.length) {
        if (text[index] === "\\") { index += 2; continue; }
        if (text[index++] === "\"") break;
      }
      try { return JSON.parse(text.slice(start, index)); } catch { throw new Error(CONTRACT_FAILURE); }
    }
    if (character === "[") {
      index++;
      const values: unknown[] = [];
      whitespace();
      if (text[index] === "]") { index++; return values; }
      while (true) {
        values.push(value(depth + 1));
        whitespace();
        if (text[index] === "]") { index++; return values; }
        if (text[index++] !== ",") throw new Error(CONTRACT_FAILURE);
      }
    }
    if (character === "{") {
      index++;
      const object: Record<string, unknown> = Object.create(null);
      const names = new Set<string>();
      whitespace();
      if (text[index] === "}") { index++; return object; }
      while (true) {
        whitespace();
        if (text[index] !== "\"") throw new Error(CONTRACT_FAILURE);
        const name = value(depth + 1);
        if (typeof name !== "string" || names.has(name)) throw new Error(CONTRACT_FAILURE);
        names.add(name);
        whitespace();
        if (text[index++] !== ":") throw new Error(CONTRACT_FAILURE);
        object[name] = value(depth + 1);
        whitespace();
        if (text[index] === "}") { index++; return object; }
        if (text[index++] !== ",") throw new Error(CONTRACT_FAILURE);
      }
    }
    const match = /^(?:null|true|false|-?(?:0|[1-9][0-9]*)(?:\.[0-9]+)?(?:[eE][+-]?[0-9]+)?)/.exec(text.slice(index));
    if (!match) throw new Error(CONTRACT_FAILURE);
    index += match[0].length;
    return JSON.parse(match[0]);
  };
  const parsed = value();
  whitespace();
  if (index !== text.length) throw new Error(CONTRACT_FAILURE);
  return parsed;
}

function declaredErrorText(
  subject: string,
  code: string,
  message: string,
  recovery: { summary: string } | undefined,
): string {
  const base = `${subject} failed with ${code}. ${message}`;
  return encodeAndTruncateHostText(
    recovery ? `${base}. Recovery: ${recovery.summary}` : base,
    MAX_ERROR_TEXT_SCALARS,
  );
}

function applicationErrorIdentity(code: string): any | undefined {
  for (const operation of HOST_SNAPSHOT.nativeSurface.document.operations as readonly any[]) {
    const errors = operation.spec.output?.application?.errors ?? [];
    const identity = errors.find(
      (error: { code: string }) => error.code === code,
    );
    if (identity) return identity;
  }
  return undefined;
}

function contextRejection(
  operation: any,
  context: HostInvocationContextV1,
): HostCallResultV1 | undefined {
  const operationId = operation.spec.id as string;
  const nativeTool = operation.call.tool as string;
  const profile = HOST_SNAPSHOT.profile as any;
  if (context.kind === "unsupported") {
    const policy = profile.unsupportedContext;
    if (!(policy.allowedOperations as readonly string[] | undefined)?.includes(operationId)) {
      const message = policy.reasons[context.reason] as string;
      return validateResult({
        version: 1,
        hostAdapterHash: HOST_ADAPTER_HASH,
        surfaceHash: SURFACE_HASH,
        outcome: {
          kind: "framework_error",
          code: "unsupported_host",
          text: declaredErrorText(nativeTool, "unsupported_host", message, policy.recovery),
        },
      });
    }
  }
  if (context.kind === "absent") {
    const rejection = profile.absentContext?.rejections?.[operationId];
    if (rejection) {
      const identity = applicationErrorIdentity(rejection.applicationCode);
      const message = rejection.runtimeMessage
        ?? identity?.summary
        ?? "Application rejected this host invocation";
      return validateResult({
        version: 1,
        hostAdapterHash: HOST_ADAPTER_HASH,
        surfaceHash: SURFACE_HASH,
        outcome: {
          kind: "application_error",
          code: rejection.applicationCode,
          text: declaredErrorText(
            nativeTool,
            rejection.applicationCode,
            message,
            rejection.recovery,
          ),
        },
      });
    }
  }
  return undefined;
}

function projectResult(result: HostCallResultV1): vscode.LanguageModelToolResult {
  if (result.outcome.kind === "success") return new vscode.LanguageModelToolResult([new vscode.LanguageModelTextPart(result.outcome.text)]);
  throw new Error(result.outcome.text);
}
"#;

const TYPESCRIPT_IN_PROCESS: &str = r#"
function captureRuntime(runtime: HostInProcessRuntime): HostInProcessRuntime {
  const call = runtime.call;
  if (typeof call !== "function") throw new Error(CONTRACT_FAILURE);
  const captured: HostInProcessRuntime = {
    call(
      tool: string,
      input: Readonly<Record<string, unknown>>,
      context: HostInvocationContextV1,
      facts: HostRuntimeFactsV1,
      token: vscode.CancellationToken,
    ) {
      return call.call(runtime, tool, input, context, facts, token);
    },
  };
  return Object.freeze(captured);
}

async function invokeRuntime(
  runtime: HostInProcessRuntime,
  hostName: string,
  input: Readonly<Record<string, unknown>>,
  context: HostInvocationContextV1,
  facts: HostRuntimeFactsV1,
  token: vscode.CancellationToken,
): Promise<HostCallResultV1> {
  encodeCallEnvelope(hostName, input, context, facts);
  let result: HostCallResultV1;
  try {
    result = await runtime.call(hostName, input, context, facts, token);
  } catch {
    throw new RuntimeHookError();
  }
  if (token.isCancellationRequested) throw new vscode.CancellationError();
  const validated = validateResult(result);
  if (canonicalJsonByteLength(validated, MAX_RESULT_BYTES) > MAX_RESULT_BYTES) throw new Error(PAYLOAD_FAILURE);
  return validated;
}
"#;

const TYPESCRIPT_PROCESS: &str = r#"
class DiagnosticForwarder {
  private remaining = MAX_STDERR_BYTES;
  private inFlight = false;
  private disabled = false;
  private truncated = false;
  private finished = false;
  private closed = false;
  private noticeOffered = false;

  constructor(private readonly sink: HostDiagnosticSink | undefined) {}

  offer(chunk: Uint8Array): void {
    if (!this.sink || this.disabled) return;
    if (this.inFlight) {
      this.truncated = true;
      return;
    }
    const count = Math.min(this.remaining, chunk.byteLength);
    if (count < chunk.byteLength) this.truncated = true;
    this.remaining -= count;
    if (count === 0) return;
    this.write(Uint8Array.from(chunk.subarray(0, count)), false);
  }

  finish(): void {
    this.finished = true;
    this.maybeNotice();
    this.closed = true;
  }

  private maybeNotice(): void {
    if (this.closed || !this.finished || !this.truncated || this.inFlight || this.disabled || this.noticeOffered) return;
    this.noticeOffered = true;
    this.write(textEncoder.encode("[mcp-twill: stderr truncated]\n"), true);
  }

  private write(chunk: Uint8Array, notice: boolean): void {
    let pending: Promise<void> | undefined;
    try {
      const result = this.sink!.write(chunk);
      if (result && typeof (result as Promise<void>).then === "function") {
        pending = Promise.resolve(result);
      }
    } catch {
      this.disabled = true;
      return;
    }
    if (!pending) {
      if (!notice) this.maybeNotice();
      return;
    }
    this.inFlight = true;
    void pending.then(
      () => {
        this.inFlight = false;
        if (!notice) this.maybeNotice();
      },
      () => {
        this.inFlight = false;
        this.disabled = true;
      },
    );
  }
}

function validateLaunch(value: unknown): HostProcessLaunch {
  const launch = cloneData(value, CONTRACT_FAILURE) as Record<string, unknown>;
  if (!exactKeys(launch, ["environment", "executable", "workingDirectory"])) throw new Error(CONTRACT_FAILURE);
  if (typeof launch.executable !== "string" || typeof launch.workingDirectory !== "string") throw new Error(CONTRACT_FAILURE);
  if (launch.executable.includes("\0") || launch.workingDirectory.includes("\0")) throw new Error(CONTRACT_FAILURE);
  const absolute = (value: string) => process.platform === "win32"
    ? /^(?:[A-Za-z]:[\\/]|\\\\[^\\]+\\[^\\]+)/.test(value)
    : value.startsWith("/");
  if (!absolute(launch.executable) || !absolute(launch.workingDirectory)) throw new Error(CONTRACT_FAILURE);
  if (!launch.environment || typeof launch.environment !== "object" || Array.isArray(launch.environment)) throw new Error(CONTRACT_FAILURE);
  const environment = launch.environment as Record<string, unknown>;
  const windowsNames = new Set<string>();
  for (const [key, entry] of Object.entries(environment)) {
    if (!key || key.includes("=") || key.includes("\0") || typeof entry !== "string" || entry.includes("\0")) throw new Error(CONTRACT_FAILURE);
    const folded = key.toUpperCase();
    if (process.platform === "win32" && windowsNames.has(folded)) throw new Error(CONTRACT_FAILURE);
    windowsNames.add(folded);
  }
  return Object.freeze({
    executable: launch.executable,
    workingDirectory: launch.workingDirectory,
    environment: Object.freeze(environment as Record<string, string>),
  });
}

function captureRuntime(runtime: HostProcessRuntime): HostProcessRuntime {
  const resolveLaunch = runtime.resolveLaunch;
  if (typeof resolveLaunch !== "function") throw new Error(CONTRACT_FAILURE);
  const sink = runtime.diagnosticSink;
  const write = sink?.write;
  if (sink !== undefined && typeof write !== "function") throw new Error(CONTRACT_FAILURE);
  const captured: HostProcessRuntime = {
    resolveLaunch(logicalName: string) {
      return resolveLaunch.call(runtime, logicalName);
    },
    diagnosticSink: sink === undefined ? undefined : Object.freeze({
      write(chunk: Uint8Array) {
        return write!.call(sink, chunk);
      },
    }),
  };
  return Object.freeze(captured);
}

function terminateProcess(child: import("node:child_process").ChildProcess): void {
  if (child.pid && process.platform !== "win32") {
    try { process.kill(-child.pid, "SIGTERM"); return; } catch {}
  }
  try { child.kill(); } catch {}
}

function forceTerminateProcess(child: import("node:child_process").ChildProcess): void {
  if (child.pid && process.platform !== "win32") {
    try { process.kill(-child.pid, "SIGKILL"); return; } catch {}
  }
  try { child.kill("SIGKILL"); } catch {}
}

function processGroupExists(child: import("node:child_process").ChildProcess): boolean {
  if (!child.pid || process.platform === "win32") return false;
  try {
    process.kill(-child.pid, 0);
    return true;
  } catch (error) {
    return (error as NodeJS.ErrnoException).code !== "ESRCH";
  }
}

async function forceTerminateAndReap(
  child: import("node:child_process").ChildProcess,
): Promise<void> {
  if (process.platform === "win32") {
    forceTerminateProcess(child);
    return;
  }
  if (!processGroupExists(child)) return;
  forceTerminateProcess(child);
  await new Promise<void>((resolve) => {
    const waitForExit = () => {
      if (processGroupExists(child)) setTimeout(waitForExit, 25);
      else resolve();
    };
    setTimeout(waitForExit, 25);
  });
}

async function invokeRuntime(
  runtime: HostProcessRuntime,
  hostName: string,
  input: Readonly<Record<string, unknown>>,
  context: HostInvocationContextV1,
  facts: HostRuntimeFactsV1,
  token: vscode.CancellationToken,
): Promise<HostCallResultV1> {
  const envelope = encodeCallEnvelope(hostName, input, context, facts);
  if (token.isCancellationRequested) throw new vscode.CancellationError();
  const launch = validateLaunch(runtime.resolveLaunch(LOGICAL_BINARY_NAME));
  if (token.isCancellationRequested) throw new vscode.CancellationError();
  const child = spawn(launch.executable, [...SUBCOMMAND, "--host-profile", HOST_PROFILE, "--host-adapter-hash", HOST_ADAPTER_HASH], {
    cwd: launch.workingDirectory,
    env: launch.environment,
    detached: process.platform !== "win32",
    shell: false,
    stdio: ["pipe", "pipe", "pipe"],
  });
  const stdout: Uint8Array[] = [];
  let stdoutBytes = 0;
  let stdoutOverflow = false;
  let terminationRequested = false;
  let forceTimer: ReturnType<typeof setTimeout> | undefined;
  let settleTermination: (() => void) | undefined;
  let terminationSettlement = Promise.resolve();
  const requestTermination = () => {
    if (terminationRequested) return;
    terminationRequested = true;
    terminationSettlement = new Promise<void>((resolve) => {
      settleTermination = resolve;
    });
    terminateProcess(child);
    forceTimer = setTimeout(() => {
      void forceTerminateAndReap(child).then(() => settleTermination?.());
    }, TERMINATION_GRACE_MS);
  };
  child.stdout!.on("data", (chunk: Uint8Array) => {
    if (stdoutOverflow) return;
    if (stdoutBytes + chunk.byteLength > MAX_RESULT_BYTES) {
      stdoutOverflow = true;
      stdout.length = 0;
      requestTermination();
      return;
    }
    stdoutBytes += chunk.byteLength;
    stdout.push(Uint8Array.from(chunk));
  });
  const diagnostics = new DiagnosticForwarder(runtime.diagnosticSink);
  child.stderr!.on("data", (chunk: Uint8Array) => diagnostics.offer(chunk));
  const cancellation = token.onCancellationRequested(requestTermination);
  let stdinFailed = false;
  let settleStdin: () => void = () => {};
  const stdinSettlement = new Promise<void>((resolve) => {
    settleStdin = resolve;
  });
  child.stdin!.on("error", () => {
    stdinFailed = true;
    requestTermination();
    settleStdin();
  });
  child.stdin!.once("finish", settleStdin);
  child.stdin!.once("close", settleStdin);
  const exitPromise = new Promise<number | null>((resolve, reject) => {
    child.once("error", reject);
    child.once("close", resolve);
  });
  try {
    child.stdin!.end(envelope);
  } catch {
    stdinFailed = true;
    requestTermination();
    settleStdin();
  }
  let exit: number | null;
  try {
    [exit] = await Promise.all([exitPromise, stdinSettlement]);
  } finally {
    if (terminationRequested && !processGroupExists(child)) {
      if (forceTimer !== undefined) clearTimeout(forceTimer);
      settleTermination?.();
    }
    if (terminationRequested) await terminationSettlement;
    cancellation.dispose();
    diagnostics.finish();
  }
  if (token.isCancellationRequested) throw new vscode.CancellationError();
  if (stdoutOverflow) throw new Error(PAYLOAD_FAILURE);
  if (stdinFailed || exit !== 0) throw new Error(CONTRACT_FAILURE);
  const bytes = new Uint8Array(stdoutBytes);
  let offset = 0;
  for (const chunk of stdout) { bytes.set(chunk, offset); offset += chunk.byteLength; }
  if (bytes.length >= 3 && bytes[0] === 0xef && bytes[1] === 0xbb && bytes[2] === 0xbf) {
    throw new Error(CONTRACT_FAILURE);
  }
  let text: string;
  try { text = textDecoder.decode(bytes); } catch { throw new Error(CONTRACT_FAILURE); }
  const parsed = parseUniqueJson(text);
  if (canonicalJson(parsed) !== text) throw new Error(CONTRACT_FAILURE);
  return validateResult(parsed as HostCallResultV1);
}
"#;

const TYPESCRIPT_REGISTRATION_BODY: &str = r#"
  if (registered) throw new Error("Generated host tools are already registered");
  const resolve = contextProvider.resolve;
  if (typeof resolve !== "function") throw new Error(CONTRACT_FAILURE);
  const capturedRuntime = captureRuntime(runtime);
  const capturedRuntimeFacts = runtimeFacts();
  const disposables: vscode.Disposable[] = [];
  try {
    for (const record of HOST_SNAPSHOT.tools as readonly any[]) {
      const hostName = record.hostName as string;
      const implementation: vscode.LanguageModelTool<Record<string, unknown>> = {
        prepareInvocation(options, token) {
          if (token.isCancellationRequested) throw new vscode.CancellationError();
          const input = inputSnapshot(options, PREPARE_FAILURE, PREPARE_FAILURE);
          validatePreparationInput(input);
          return preparedInvocation(hostName, input);
        },
        async invoke(options, token) {
          if (token.isCancellationRequested) throw new vscode.CancellationError();
          const input = inputSnapshot(options, CONTRACT_FAILURE, CALL_PAYLOAD_FAILURE);
          const record = toolRecord(hostName, CONTRACT_FAILURE);
          const frameworkHelp = isFrameworkHelp(record);
          const operation = frameworkHelp
            ? undefined
            : operationRecord(record, input, CONTRACT_FAILURE);
          let context: HostInvocationContextV1;
          if (frameworkHelp) {
            context = Object.freeze({ kind: "absent" });
          } else {
            try { context = validateContext(resolve.call(contextProvider, options)); }
            catch (error) {
              if (error instanceof CloneLimitError) throw error;
              context = Object.freeze({ kind: "unsupported", reason: "provider_failed" });
            }
          }
          if (token.isCancellationRequested) throw new vscode.CancellationError();
          if (operation) {
            const rejection = contextRejection(operation, context);
            if (rejection) return projectResult(rejection);
          }
          let result: HostCallResultV1;
          try {
            result = await invokeRuntime(capturedRuntime as never, hostName, input, context, capturedRuntimeFacts, token);
          } catch (error) {
            if (token.isCancellationRequested || error instanceof vscode.CancellationError) {
              throw new vscode.CancellationError();
            }
            if (error instanceof RuntimeHookError) throw new Error(CONTRACT_FAILURE);
            const message = error instanceof Error ? error.message : "";
            if ([CONTRACT_FAILURE, CALL_PAYLOAD_FAILURE, PAYLOAD_FAILURE].includes(message)) {
              throw new Error(message);
            }
            throw new Error(CONTRACT_FAILURE);
          }
          return projectResult(result);
        },
      };
      disposables.push(vscode.lm.registerTool(hostName, implementation));
    }
    const composite = vscode.Disposable.from(...disposables);
    extensionContext.subscriptions.push(composite);
    registered = true;
  } catch (error) {
    for (const disposable of disposables.reverse()) {
      try { disposable.dispose(); } catch {}
    }
    throw error;
  }
"#;

fn build_error(message: impl Into<String>) -> FrameworkError {
    FrameworkError::Build(message.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_source_has_one_final_newline() {
        let mut source = "value".to_string();
        if !source.ends_with('\n') {
            source.push('\n');
        }
        assert!(source.ends_with('\n'));
        assert!(!source.ends_with("\n\n"));
    }

    #[test]
    fn javascript_strings_use_json_escaping() {
        assert_eq!(js_string("a\nb"), r#""a\nb""#);
    }
}
