import { spawnSync } from "node:child_process";
import { EventEmitter } from "node:events";
import {
  mkdirSync,
  mkdtempSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import Module, { createRequire } from "node:module";
import { tmpdir } from "node:os";
import { join } from "node:path";
import ts from "typescript";

const root = new URL("../../../../", import.meta.url);
const directory = mkdtempSync(join(tmpdir(), "mcp-twill-host-typescript-"));
const generatedSources = new Map();

try {
  for (const transport of ["in-process", "process"]) {
    const generated = spawnSync(
      "cargo",
      [
        "run",
        "--quiet",
        "--manifest-path",
        new URL("Cargo.toml", root).pathname,
        "-p",
        "mcp-twill-host",
        "--example",
        "generated_vscode_adapter",
        "--",
        transport,
      ],
      { encoding: "utf8" },
    );
    if (generated.status !== 0) {
      throw new Error(generated.stderr || `generator exited ${generated.status}`);
    }
    generatedSources.set(transport, generated.stdout);
    const source = join(directory, `${transport}.ts`);
    writeFileSync(source, generated.stdout);
    const program = ts.createProgram([source], {
      strict: true,
      noEmit: true,
      target: ts.ScriptTarget.ES2022,
      module: ts.ModuleKind.NodeNext,
      moduleResolution: ts.ModuleResolutionKind.NodeNext,
      types: ["node", "vscode"],
      skipLibCheck: false,
    });
    const diagnostics = ts.getPreEmitDiagnostics(program);
    if (diagnostics.length > 0) {
      throw new Error(
        ts.formatDiagnosticsWithColorAndContext(diagnostics, {
          getCanonicalFileName: (name) => name,
          getCurrentDirectory: () => process.cwd(),
          getNewLine: () => "\n",
        }),
      );
    }
  }

  const vscodeDirectory = join(directory, "node_modules", "vscode");
  mkdirSync(vscodeDirectory, { recursive: true });
  writeFileSync(
    join(vscodeDirectory, "package.json"),
    JSON.stringify({ name: "vscode", version: "1.128.0", main: "index.cjs" }),
  );
  writeFileSync(
    join(vscodeDirectory, "index.cjs"),
    `
class CancellationError extends Error {}
class LanguageModelTextPart { constructor(value) { this.value = value; } }
class LanguageModelToolResult { constructor(content) { this.content = content; } }
const state = { registrations: [], registerTool: undefined };
const lm = {
  registerTool(name, implementation) {
    if (state.registerTool) return state.registerTool(name, implementation);
    const disposable = { dispose() {} };
    state.registrations.push({ name, implementation, disposable });
    return disposable;
  },
};
const Disposable = {
  from(...disposables) {
    return { dispose() { for (const disposable of disposables) disposable.dispose(); } };
  },
};
module.exports = {
  version: "1.128.0",
  CancellationError,
  LanguageModelTextPart,
  LanguageModelToolResult,
  Disposable,
  lm,
  __state: state,
};
`,
  );
  const inProcessSource = generatedSources.get("in-process");
  if (!inProcessSource) throw new Error("missing generated in-process source");
  const compileRuntimeModule = (name, source = inProcessSource) => {
    const javascript = ts.transpileModule(source, {
      compilerOptions: {
        target: ts.ScriptTarget.ES2022,
        module: ts.ModuleKind.CommonJS,
      },
    }).outputText;
    const path = join(directory, `${name}.cjs`);
    writeFileSync(path, javascript);
    return path;
  };
  const requireFromFixture = createRequire(join(directory, "fixture.cjs"));
  const vscode = requireFromFixture("vscode");
  const token = (cancelled = false) => ({
    isCancellationRequested: cancelled,
    onCancellationRequested() {
      return { dispose() {} };
    },
  });
  const hashes = {
    host: /const HOST_ADAPTER_HASH = "([^"]+)"/.exec(inProcessSource)?.[1],
    surface: /const SURFACE_HASH = "([^"]+)"/.exec(inProcessSource)?.[1],
  };
  if (!hashes.host || !hashes.surface) throw new Error("missing generated hashes");

  const first = requireFromFixture(compileRuntimeModule("runtime-success"));
  let providerCalls = 0;
  let runtimeCalls = 0;
  let providerContext = { kind: "absent" };
  const provider = {
    marker: "provider",
    resolve() {
      if (this.marker !== "provider") throw new Error("lost provider receiver");
      providerCalls++;
      return providerContext;
    },
  };
  const runtime = {
    marker: "runtime",
    async call(tool, input, context, facts) {
      if (this.marker !== "runtime") throw new Error("lost runtime receiver");
      runtimeCalls++;
      if (tool !== "item_get" || input.id !== "42" || context.kind !== "absent") {
        throw new Error("unexpected generated invocation");
      }
      if (facts.kind !== "vs_code" || facts.engineVersion?.minor !== 128) {
        throw new Error("runtime facts were not captured");
      }
      return {
        version: 1,
        hostAdapterHash: hashes.host,
        surfaceHash: hashes.surface,
        outcome: { kind: "success", text: '{"id":"42","value":"found"}' },
      };
    },
  };
  const extension = { subscriptions: [] };
  first.registerGeneratedHostTools(extension, provider, runtime);
  if (providerCalls !== 0 || runtimeCalls !== 0 || extension.subscriptions.length !== 1) {
    throw new Error("registration performed runtime work or lost composite ownership");
  }
  const registration = vscode.__state.registrations.at(-1);
  const prepared = registration.implementation.prepareInvocation(
    { input: { id: "42" } },
    token(),
  );
  if (!prepared.invocationMessage || providerCalls !== 0 || runtimeCalls !== 0) {
    throw new Error("preparation performed runtime work");
  }
  let oversizedPreparationFailed = false;
  try {
    registration.implementation.prepareInvocation(
      { input: { id: "x".repeat(70_000) } },
      token(),
    );
  } catch (error) {
    oversizedPreparationFailed =
      error.message === "Generated host adapter could not prepare this invocation";
  }
  if (!oversizedPreparationFailed || providerCalls !== 0 || runtimeCalls !== 0) {
    throw new Error("oversized preparation input reached a host hook");
  }
  const nested = (count) => {
    let value = "leaf";
    for (let index = 0; index < count; index++) value = { next: value };
    return value;
  };
  registration.implementation.prepareInvocation(
    { input: { id: "42", nested: nested(127) } },
    token(),
  );
  let overDepthPreparationFailed = false;
  try {
    registration.implementation.prepareInvocation(
      { input: { id: "42", nested: nested(128) } },
      token(),
    );
  } catch (error) {
    overDepthPreparationFailed =
      error.message === "Generated host adapter could not prepare this invocation";
  }
  if (!overDepthPreparationFailed || providerCalls !== 0 || runtimeCalls !== 0) {
    throw new Error("over-depth preparation input reached a host hook");
  }
  const numericLookingProperty = [];
  Object.defineProperty(numericLookingProperty, "4294967295", {
    enumerable: true,
    value: "not-an-array-index",
  });
  let numericLookingPropertyFailed = false;
  try {
    await registration.implementation.invoke(
      { input: { id: "42", values: numericLookingProperty } },
      token(),
    );
  } catch (error) {
    numericLookingPropertyFailed =
      error.message === "Generated host adapter received an invalid result envelope";
  }
  if (!numericLookingPropertyFailed || providerCalls !== 0 || runtimeCalls !== 0) {
    throw new Error("numeric-looking array property crossed the snapshot boundary");
  }
  let oversizedArrayFailed = false;
  try {
    await registration.implementation.invoke(
      { input: { id: "42", values: Array.from({ length: 70_000 }, () => "value") } },
      token(),
    );
  } catch (error) {
    oversizedArrayFailed =
      error.message === "Generated host call exceeds its configured byte limit";
  }
  if (!oversizedArrayFailed || providerCalls !== 0 || runtimeCalls !== 0) {
    throw new Error("oversized array reached a host hook");
  }
  provider.resolve = () => {
    throw new Error("replacement provider method was observed");
  };
  runtime.call = async () => {
    throw new Error("replacement runtime method was observed");
  };
  const result = await registration.implementation.invoke(
    { input: { id: "42" } },
    token(),
  );
  if (
    providerCalls !== 1
    || runtimeCalls !== 1
    || result.content[0].value !== '{"id":"42","value":"found"}'
  ) {
    throw new Error("generated invocation did not preserve captured hooks");
  }
  providerContext = { kind: "unsupported", reason: "unknown_token_shape" };
  let unsupportedContextFailed = false;
  try {
    await registration.implementation.invoke(
      { input: { id: "42" } },
      token(),
    );
  } catch (error) {
    unsupportedContextFailed =
      error.message ===
      "item_get failed with unsupported_host. This host cannot supply invocation context";
  }
  if (!unsupportedContextFailed || providerCalls !== 2 || runtimeCalls !== 1) {
    throw new Error("known context rejection reached the runtime hook");
  }
  providerContext = { kind: "absent" };
  const shared = { value: "copied" };
  let repeatedIdentityFailed = false;
  try {
    await registration.implementation.invoke(
      { input: { id: "42", left: shared, right: shared } },
      token(),
    );
  } catch (error) {
    repeatedIdentityFailed =
      error.message === "Generated host adapter received an invalid result envelope";
  }
  if (!repeatedIdentityFailed || providerCalls !== 2 || runtimeCalls !== 1) {
    throw new Error("repeated source identity crossed the snapshot boundary");
  }
  let duplicateFailed = false;
  try {
    first.registerGeneratedHostTools({ subscriptions: [] }, provider, runtime);
  } catch {
    duplicateFailed = true;
  }
  if (!duplicateFailed) throw new Error("second registration succeeded");

  vscode.__state.registrations.length = 0;
  const retry = requireFromFixture(compileRuntimeModule("runtime-retry"));
  let failRegistration = true;
  vscode.__state.registerTool = (name, implementation) => {
    if (failRegistration) throw new Error("activation failed");
    const disposable = { dispose() {} };
    vscode.__state.registrations.push({ name, implementation, disposable });
    return disposable;
  };
  let activationFailed = false;
  try {
    retry.registerGeneratedHostTools({ subscriptions: [] }, provider, runtime);
  } catch (error) {
    activationFailed = error.message === "activation failed";
  }
  if (!activationFailed) throw new Error("registration did not preserve activation failure");
  failRegistration = false;
  const retriedExtension = { subscriptions: [] };
  retry.registerGeneratedHostTools(retriedExtension, provider, runtime);
  if (retriedExtension.subscriptions.length !== 1) {
    throw new Error("registration did not remain retryable");
  }
  const cancelledRegistration = vscode.__state.registrations.at(-1);
  let cancelled = false;
  try {
    await cancelledRegistration.implementation.invoke(
      { input: { id: "42" } },
      token(true),
    );
  } catch (error) {
    cancelled = error instanceof vscode.CancellationError;
  }
  if (!cancelled || providerCalls !== 2 || runtimeCalls !== 1) {
    throw new Error("pre-dispatch cancellation reached a host hook");
  }

  const processSource = generatedSources.get("process");
  if (!processSource) throw new Error("missing generated process source");
  const processHashes = {
    host: /const HOST_ADAPTER_HASH = "([^"]+)"/.exec(processSource)?.[1],
    surface: /const SURFACE_HASH = "([^"]+)"/.exec(processSource)?.[1],
  };
  if (!processHashes.host || !processHashes.surface) {
    throw new Error("missing generated process hashes");
  }
  let spawnRecord;
  let resumeBackpressure = () => {};
  class FakeChild extends EventEmitter {
    constructor() {
      super();
      this.pid = 987_654_321;
      this.exitCode = null;
      this.signalCode = null;
      this.stdout = new EventEmitter();
      this.stderr = new EventEmitter();
      this.stdin = new EventEmitter();
      this.stdin.end = (bytes) => {
        const call = JSON.parse(new TextDecoder().decode(bytes));
        if (call.arguments.id === "stdin-error") {
          queueMicrotask(() => this.stdin.emit("error", new Error("EPIPE")));
          return;
        }
        queueMicrotask(() => {
          this.stdin.emit("finish");
          if (call.arguments.id === "cancel") return;
          if (call.arguments.id === "backpressure") {
            this.stderr.emit("data", Uint8Array.from([1]));
            this.stderr.emit("data", new Uint8Array(70_000));
            resumeBackpressure = () => {
              this.stderr.emit("data", Uint8Array.from([9]));
              this.complete(call);
            };
            return;
          }
          this.stderr.emit("data", Uint8Array.from([1, 2, 3]));
          this.stderr.emit("data", Uint8Array.from([4, 5, 6]));
          this.complete(call);
        });
      };
    }

    complete(call) {
      const result = {
        hostAdapterHash: call.hostAdapterHash,
        outcome: { kind: "success", text: '{"id":"42","value":"found"}' },
        surfaceHash: call.surfaceHash,
        version: 1,
      };
      this.stdout.emit("data", new TextEncoder().encode(JSON.stringify(result)));
      this.exitCode = 0;
      this.emit("close", 0);
    }

    kill(signal) {
      this.signalCode = signal ?? "SIGTERM";
      queueMicrotask(() => this.emit("close", null));
      return true;
    }
  }
  const originalLoad = Module._load;
  Module._load = function load(request, parent, isMain) {
    if (request === "node:child_process") {
      return {
        spawn(executable, args, options) {
          const child = new FakeChild();
          spawnRecord = { executable, args, options, child };
          return child;
        },
      };
    }
    return originalLoad.call(this, request, parent, isMain);
  };
  let processModule;
  try {
    processModule = requireFromFixture(
      compileRuntimeModule("runtime-process", processSource),
    );
  } finally {
    Module._load = originalLoad;
  }
  vscode.__state.registrations.length = 0;
  vscode.__state.registerTool = undefined;
  let processProviderCalls = 0;
  let processResolverCalls = 0;
  let diagnosticCalls = 0;
  const diagnosticChunks = [];
  let settleDiagnostic;
  let diagnosticMode = "pending";
  let firstBackpressureWrite = true;
  const processExtension = { subscriptions: [] };
  processModule.registerGeneratedHostTools(
    processExtension,
    {
      resolve() {
        processProviderCalls++;
        return { kind: "absent" };
      },
    },
    {
      resolveLaunch(logicalName) {
        processResolverCalls++;
        if (logicalName !== "bin/generated-host-example") {
          throw new Error("unexpected logical binary name");
        }
        return {
          executable: process.execPath,
          workingDirectory: directory,
          environment: {},
        };
      },
      diagnosticSink: {
        write(chunk) {
          diagnosticCalls++;
          diagnosticChunks.push([...chunk]);
          if (diagnosticMode === "throwing-then") {
            return Object.defineProperty({}, "then", {
              get() {
                throw new Error("hostile then getter");
              },
            });
          }
          if (diagnosticMode === "backpressure" && !firstBackpressureWrite) {
            return;
          }
          firstBackpressureWrite = false;
          return new Promise((resolve) => {
            settleDiagnostic = resolve;
          });
        },
      },
    },
  );
  const processRegistration = vscode.__state.registrations.at(-1);
  const processResult = await processRegistration.implementation.invoke(
    { input: { id: "42" } },
    token(),
  );
  if (
    processProviderCalls !== 1
    || processResolverCalls !== 1
    || diagnosticCalls !== 1
    || processResult.content[0].value !== '{"id":"42","value":"found"}'
  ) {
    throw new Error("generated process invocation did not complete its closed transport");
  }
  settleDiagnostic();
  await Promise.resolve();
  await Promise.resolve();
  if (diagnosticCalls !== 1) {
    throw new Error("call-local diagnostics continued after transport cleanup");
  }
  diagnosticMode = "throwing-then";
  const hostileDiagnosticResult = await processRegistration.implementation.invoke(
    { input: { id: "diagnostic-thenable" } },
    token(),
  );
  if (
    diagnosticCalls !== 2
    || hostileDiagnosticResult.content[0].value !== '{"id":"42","value":"found"}'
  ) {
    throw new Error("hostile diagnostic thenable escaped the process transport");
  }
  let stdinErrorFailed = false;
  try {
    await processRegistration.implementation.invoke(
      { input: { id: "stdin-error" } },
      token(),
    );
  } catch (error) {
    stdinErrorFailed =
      error.message === "Generated host adapter received an invalid result envelope";
  }
  if (!stdinErrorFailed) {
    throw new Error("child stdin error escaped the process transport");
  }
  diagnosticMode = "backpressure";
  firstBackpressureWrite = true;
  const backpressured = processRegistration.implementation.invoke(
    { input: { id: "backpressure" } },
    token(),
  );
  await Promise.resolve();
  await Promise.resolve();
  settleDiagnostic();
  await Promise.resolve();
  await Promise.resolve();
  resumeBackpressure();
  await backpressured;
  if (!diagnosticChunks.some((chunk) => chunk.length === 1 && chunk[0] === 9)) {
    throw new Error("dropped stderr consumed the offered-byte allowance");
  }
  if (
    spawnRecord.executable !== process.execPath
    || spawnRecord.options.shell !== false
    || spawnRecord.options.cwd !== directory
    || spawnRecord.options.detached !== (process.platform !== "win32")
    || spawnRecord.args.at(-4) !== "--host-profile"
    || spawnRecord.args.at(-2) !== "--host-adapter-hash"
  ) {
    throw new Error("generated process invocation drifted from the compiled launch");
  }
  let processCancelled = false;
  let cancelProcess = () => {};
  const processToken = {
    get isCancellationRequested() {
      return processCancelled;
    },
    onCancellationRequested(listener) {
      cancelProcess = () => {
        processCancelled = true;
        listener();
      };
      return { dispose() {} };
    },
  };
  const pendingCancellation = processRegistration.implementation.invoke(
    { input: { id: "cancel" } },
    processToken,
  );
  await Promise.resolve();
  const originalProcessKill = process.kill;
  const originalSetTimeout = globalThis.setTimeout;
  const groupSignals = [];
  let groupAlive = process.platform !== "win32";
  if (process.platform !== "win32") {
    process.kill = (pid, signal) => {
      if (pid !== -spawnRecord.child.pid) return originalProcessKill(pid, signal);
      if (signal === 0) {
        if (groupAlive) return true;
        const error = new Error("no such process group");
        error.code = "ESRCH";
        throw error;
      }
      groupSignals.push(signal);
      if (signal === "SIGTERM") {
        queueMicrotask(() => spawnRecord.child.emit("close", null));
      }
      if (signal === "SIGKILL") groupAlive = false;
      return true;
    };
    globalThis.setTimeout = (callback, delay, ...args) =>
      originalSetTimeout(callback, Math.min(delay, 10), ...args);
  }
  cancelProcess();
  let processRaisedCancellation = false;
  try {
    await pendingCancellation;
    await new Promise((resolve) => originalSetTimeout(resolve, 50));
  } catch (error) {
    processRaisedCancellation = error instanceof vscode.CancellationError;
    await new Promise((resolve) => originalSetTimeout(resolve, 50));
  } finally {
    process.kill = originalProcessKill;
    globalThis.setTimeout = originalSetTimeout;
  }
  if (!processRaisedCancellation || spawnRecord.options.detached !== (process.platform !== "win32")) {
    throw new Error("generated process cancellation did not reap its wrapper");
  }
  if (
    process.platform !== "win32"
    && (!groupSignals.includes("SIGTERM") || !groupSignals.includes("SIGKILL") || groupAlive)
  ) {
    throw new Error("generated process cancellation did not reap its descendant group");
  }
} finally {
  rmSync(directory, { recursive: true, force: true });
}
