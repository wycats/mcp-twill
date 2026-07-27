import {
  useMemo,
  useReducer,
  useRef,
  type KeyboardEvent,
} from "react";
import type {
  ConfirmationKind,
  Destination,
  LoadedEvidence,
  PrivateContext,
  ProjectionName,
  ServingProfile,
  TitleRule,
  VariantSelection,
} from "./evidence/types";
import { projections } from "./evidence/types";
import { findVariant, prettyJson } from "./evidence/adapter";
import {
  displayedFactId,
  initialState,
  mismatches,
  seedHandwrittenDrift,
  workbenchReducer,
  type GuideStepId,
} from "./state";
import { CausalThreads } from "./components/CausalThreads";
import { CopyButton } from "./components/CopyButton";
import { DeclarationCode } from "./components/DeclarationCode";
import { GuidedProof } from "./components/GuidedProof";
import { ProjectionPanel } from "./components/ProjectionPanel";
import styles from "./App.module.css";

const repository = "https://github.com/wycats/mcp-twill";
const repositorySource = `${repository}/blob/main/`;
const frozenEvidenceRevision =
  "ab18ad46727b41210c542cbb7c950f174c739d2d";

interface AppProps {
  evidence: LoadedEvidence;
}

function shortHash(value: string): string {
  return value.length > 18 ? `${value.slice(0, 10)}…${value.slice(-6)}` : value;
}

function selectValue(
  dispatch: React.Dispatch<Parameters<typeof workbenchReducer>[1]>,
  control: keyof VariantSelection,
  value: string,
  factId: string,
) {
  switch (control) {
    case "titleRule":
      if (value === "unconstrained" || value === "nonEmpty") {
        dispatch({
          type: "select",
          control,
          value: value as TitleRule,
          factId,
        });
      }
      break;
    case "destination":
      if (value === "local" || value === "remote") {
        dispatch({
          type: "select",
          control,
          value: value as Destination,
          factId,
        });
      }
      break;
    case "confirmation":
      if (value === "generic" || value === "titleInterpolated") {
        dispatch({
          type: "select",
          control,
          value: value as ConfirmationKind,
          factId,
        });
      }
      break;
    case "privateContext":
      if (value === "none" || value === "conversationIdentity") {
        dispatch({
          type: "select",
          control,
          value: value as PrivateContext,
          factId,
        });
      }
      break;
  }
}

function GuideStep({
  number,
  title,
  children,
  action,
  actionLabel = "Show this",
  active = false,
  status,
}: {
  number: string;
  title: string;
  children: React.ReactNode;
  action: (trigger: HTMLButtonElement) => void;
  actionLabel?: string;
  active?: boolean;
  status?: string | undefined;
}) {
  return (
    <section
      className={`${styles.guideStep} ${active ? styles.guideStepActive : ""}`}
      data-active={active}
    >
      <p className={styles.eyebrow}>{number}</p>
      <h2>{title}</h2>
      <p>{children}</p>
      <p
        className={styles.guideStatus}
        data-visible={Boolean(status)}
        aria-hidden={!status}
      >
        {status ?? "\u00a0"}
      </p>
      <button
        type="button"
        aria-controls="guided-proof"
        aria-current={active ? "step" : undefined}
        onClick={(event) => action(event.currentTarget)}
      >
        {actionLabel}
      </button>
    </section>
  );
}

function SourceLink({
  path,
  revision = "main",
  children,
}: {
  path: string;
  revision?: string;
  children: React.ReactNode;
}) {
  return (
    <a href={`${repository}/blob/${revision}/${path}`}>
      {children}
    </a>
  );
}

function VblFieldStudy({ evidence }: AppProps) {
  const historical = evidence.manifest.files.find((file) =>
    file.path.endsWith("vbl/catalog-measurement.json"),
  );
  const fixture = evidence.manifest.files.find((file) =>
    file.path.endsWith("vbl/v0.4.9-manifest.json"),
  );

  return (
    <section className={styles.fieldStudy} aria-labelledby="vbl-field-study">
      <div>
        <p className={styles.eyebrow}>Field study</p>
        <h2 id="vbl-field-study">A frozen adoption case</h2>
        <p className={styles.lede}>
          Visible Browser Lab makes the ownership boundary concrete. Twill owns
          agreement among the catalog-derived help, schema, tools, and
          presentation. VBL keeps the broker, leases, browser operations, and
          application policy.
        </p>
      </div>
      <div className={styles.fieldStudyEvidence}>
        <div>
          <p className={styles.provenanceKind}>Ownership outcome · essay explanation</p>
          <dl className={styles.ownershipMatrix}>
            <div>
              <dt>Moved to Twill</dt>
              <dd>
                <strong>Catalog-derived agreement</strong>
                <span>
                  Help, schemas, tool surfaces, and presentation values now
                  share one authority.
                </span>
              </dd>
            </div>
            <div>
              <dt>Stayed in VBL</dt>
              <dd>
                <strong>Application behavior and policy</strong>
                <span>
                  The broker, leases, browser operations, and product policy
                  remain VBL-owned.
                </span>
              </dd>
            </div>
            <div>
              <dt>Verified by</dt>
              <dd>
                <strong>Frozen evidence plus contract support</strong>
                <span>
                  The measurement and release fixture below stay distinct;{" "}
                  <SourceLink
                    path="crates/mcp-twill/tests/native_surfaces.rs#L1966-L2025"
                    revision={frozenEvidenceRevision}
                  >
                    Twill’s VBL projection contract test
                  </SourceLink>{" "}
                  checks the boundary.
                </span>
              </dd>
            </div>
          </dl>
        </div>
        <div className={styles.provenanceColumns}>
          <article>
            <p className={styles.provenanceKind}>Historical measurement</p>
            <h3>Pre-port observation</h3>
            <p>
              A measured catalog snapshot with its own acquisition context. It is
              evidence of the starting point, not a newly pinned baseline.
            </p>
            <p className={styles.hashLine}>
              {historical
                ? `${historical.bytes} bytes · ${shortHash(historical.sha256)}`
                : "Evidence copy unavailable"}
            </p>
            <SourceLink
              path="docs/adoption/visible-browser-lab/baseline/catalog-measurement.json"
              revision={frozenEvidenceRevision}
            >
              Inspect immutable historical source
            </SourceLink>
            <details className={styles.fieldEvidence}>
              <summary>Frozen evidence copy</summary>
              <CopyButton
                value={prettyJson(evidence.vbl.historicalMeasurement)}
                label="Copy measurement JSON"
              />
              <pre tabIndex={0}>
                <code>{prettyJson(evidence.vbl.historicalMeasurement)}</code>
              </pre>
            </details>
          </article>
          <article>
            <p className={styles.provenanceKind}>Frozen release fixture</p>
            <h3>VBL v0.4.9 contract evidence</h3>
            <p>
              A byte-for-byte fixture used by Twill’s contract tests. Its release
              identity remains separate from the earlier measurement.
            </p>
            <p className={styles.hashLine}>
              {fixture
                ? `${fixture.bytes} bytes · ${shortHash(fixture.sha256)}`
                : "Evidence copy unavailable"}
            </p>
            <SourceLink
              path="crates/mcp-twill/tests/fixtures/vbl/v0.4.9/manifest.json"
              revision={frozenEvidenceRevision}
            >
              Inspect immutable frozen fixture
            </SourceLink>
            <details className={styles.fieldEvidence}>
              <summary>Frozen evidence copy</summary>
              <CopyButton
                value={evidence.vbl.frozenFixtureManifestRaw}
                label="Copy fixture JSON"
              />
              <pre tabIndex={0}>
                <code>{evidence.vbl.frozenFixtureManifestRaw}</code>
              </pre>
            </details>
          </article>
        </div>
        <p className={styles.ownershipNote}>
          These links resolve to the reviewed commit whose bytes match the two
          tracked evidence hashes. The older gap map is not used as current
          implementation authority.
        </p>
      </div>
    </section>
  );
}

export function App({ evidence }: AppProps) {
  const { bundle, manifest } = evidence;
  const [state, dispatch] = useReducer(
    workbenchReducer,
    initialState(bundle.defaults.selection, bundle.defaults.profile),
  );
  const wovenRef = useRef<HTMLDivElement>(null);
  const handwrittenButtonRef = useRef<HTMLButtonElement>(null);
  const derivedButtonRef = useRef<HTMLButtonElement>(null);
  const variant = useMemo(
    () => findVariant(bundle.variants, state.selection),
    [bundle.variants, state.selection],
  );
  const activeFactId = displayedFactId(state);
  const omittedDeclarationFact = variant.declaration.facts.find(
    (fact) => fact.codePresence === "omitted",
  );
  const activeFactIsOmitted =
    omittedDeclarationFact?.id === activeFactId;
  const drift = mismatches(variant, state.overrides);
  const driftTargetIds = useMemo(
    () => new Set(drift.map((target) => target.id)),
    [drift],
  );

  const controlFactIds = useMemo(
    () =>
      new Map(
        bundle.controls.map((control) => [control.id, control.factId] as const),
      ),
    [bundle.controls],
  );
  const titleFactId = controlFactIds.get("titleRule")!;
  const destinationFactId = controlFactIds.get("destination")!;
  const privateContextFactId = controlFactIds.get("privateContext")!;

  function showSelection(
    selection: VariantSelection,
    factId: string | null,
    guideStep: GuideStepId,
  ) {
    if (state.mode === "handwritten") dispatch({ type: "restore" });
    dispatch({ type: "replaceSelection", selection, factId, guideStep });
  }

  function enterHandwritten() {
    dispatch({
      type: "enterHandwritten",
      overrides: seedHandwrittenDrift(variant),
      factId: titleFactId,
    });
  }

  function restoreAuthority(trigger?: HTMLButtonElement | null) {
    dispatch({ type: "restore" });
    requestAnimationFrame(() => {
      if (trigger?.isConnected) {
        trigger.focus();
      } else {
        derivedButtonRef.current?.focus();
      }
    });
  }

  function handleProjectionKeys(
    event: KeyboardEvent<HTMLButtonElement>,
    projection: ProjectionName,
  ) {
    if (!["ArrowLeft", "ArrowRight", "Home", "End"].includes(event.key)) {
      return;
    }
    event.preventDefault();
    const current = projections.indexOf(projection);
    const next =
      event.key === "Home"
        ? 0
        : event.key === "End"
          ? projections.length - 1
          : (current + (event.key === "ArrowRight" ? 1 : -1) + projections.length) %
            projections.length;
    const target = projections[next] ?? "help";
    dispatch({ type: "projection", projection: target });
    requestAnimationFrame(() => {
      document.querySelector<HTMLButtonElement>(`[data-tab="${target}"]`)?.focus();
    });
  }

  const defaults = bundle.defaults.selection;
  const withTitleLimit = { ...defaults, titleRule: "nonEmpty" as const };
  const withRemote = { ...withTitleLimit, destination: "remote" as const };
  const withIdentity = {
    ...withRemote,
    privateContext: "conversationIdentity" as const,
  };

  return (
    <>
      <header className={styles.masthead}>
        <div>
          <p className={styles.kicker}>Twill · an explorable technical essay</p>
          <h1>A Command, Woven.</h1>
        </div>
        <p>
          One authoritative command declaration becomes every interpretation of
          an operation—and keeps them truthful together.
        </p>
      </header>

      <main>
        <section className={styles.experience} aria-labelledby="workbench-title">
          <nav className={styles.narrative} aria-label="Guided essay">
            <GuideStep
              number="01"
              title="One command makes five promises"
              action={() => showSelection(defaults, null, 1)}
              actionLabel="See the five promises"
              active={state.guideStep === 1}
              status={
                state.guideStep === 1
                  ? "Five projections staged below."
                  : undefined
              }
            >
              <code>issues create</code> becomes Help for people, Schema for
              runtime validation, an MCP tool for agents, Confirmation
              presentation, and Host rendering.
            </GuideStep>
            <GuideStep
              number="02"
              title="Change the rule once"
              action={() => showSelection(withTitleLimit, titleFactId, 2)}
              actionLabel="Change the title rule"
              active={state.guideStep === 2}
              status={
                state.guideStep === 2
                  ? "Title consequences staged below."
                  : undefined
              }
            >
              Require a non-empty title. Every promise should now tell the same
              new truth: <code>minLength: 1</code>.
            </GuideStep>
            <GuideStep
              number="03"
              title="Behavior is part of the promise"
              action={() => showSelection(withRemote, destinationFactId, 3)}
              actionLabel="Add network access"
              active={state.guideStep === 3}
              status={
                state.guideStep === 3
                  ? "Effect consequences staged below."
                  : undefined
              }
            >
              Add network access. Permissions, authorization, annotations, and
              host warnings must change together.
            </GuideStep>
            <GuideStep
              number="04"
              title="Handwritten copies eventually disagree"
              action={enterHandwritten}
              actionLabel="Introduce drift"
              active={state.guideStep === 4}
              status={
                state.guideStep === 4
                  ? `${drift.length} contradictions staged below.`
                  : undefined
              }
            >
              Let each consumer maintain its own interpretation and watch one
              command start contradicting itself.
            </GuideStep>
            <GuideStep
              number="05"
              title="Give truth one home"
              action={restoreAuthority}
              actionLabel="Restore from catalog"
              active={state.guideStep === 5}
              status={
                state.guideStep === 5
                  ? "Authority and build gates restored."
                  : undefined
              }
            >
              Restore the catalog, then follow that same authority through
              selection, validation, authorization, dispatch, and result.
            </GuideStep>
            <GuideStep
              number="06"
              title="Some inputs should never become public"
              action={() => showSelection(withIdentity, privateContextFactId, 6)}
              actionLabel="Supply private context"
              active={state.guideStep === 6}
              status={
                state.guideStep === 6
                  ? "Private-context proof staged below."
                  : undefined
              }
            >
              Conversation identity can shape the right decision without
              appearing in arguments, help, logs, or results.
            </GuideStep>
            <GuideStep
              number="07"
              title="One operation, two public call shapes"
              action={() => {
                if (state.mode === "handwritten") {
                  dispatch({ type: "restore" });
                }
                dispatch({ type: "compareMcp", active: !state.compareMcp });
              }}
              actionLabel={
                state.compareMcp
                  ? `Return to ${state.profile === "compact" ? "compact lanes" : "native tool"}`
                  : "Compare the two tool shapes"
              }
              active={state.guideStep === 7}
              status={
                state.compareMcp
                  ? "MCP comparison staged below."
                  : undefined
              }
            >
              Compact shares a small <code>run*</code> vocabulary and carries
              the command through <code>command + args</code>. Native exposes{" "}
              <code>{variant.mcpSurfaceComparison.native.toolName}</code>{" "}
              with <code>title + body</code>. Both resolve to{" "}
              <code>{variant.mcpSurfaceComparison.operationId}</code>.
            </GuideStep>
          </nav>

          <section className={styles.workbench} data-testid="workbench">
            <header className={styles.workbenchHeader}>
              <div>
                <p className={styles.eyebrow}>Persistent specimen</p>
                <h2 id="workbench-title">
                  <code>issues create</code>
                </h2>
                <p className={styles.workbenchDefinition}>
                  <strong>Catalog</strong>: Twill’s authoritative runtime model.
                  Semantic values come from it; host layout is site-owned.
                </p>
              </div>
              <div className={styles.displaySwitches}>
                <div className={styles.displayControl}>
                  <span>Authority</span>
                  <div
                    className={styles.authoritySwitch}
                    aria-label="Authority mode"
                  >
                    <button
                      ref={derivedButtonRef}
                      type="button"
                      aria-pressed={state.mode === "derived"}
                      onClick={(event) => restoreAuthority(event.currentTarget)}
                    >
                      Derived
                    </button>
                    <button
                      ref={handwrittenButtonRef}
                      type="button"
                      aria-pressed={state.mode === "handwritten"}
                      onClick={enterHandwritten}
                    >
                      Handwritten
                    </button>
                  </div>
                </div>
                <div className={styles.displayControl}>
                  <span id="mcp-surface-label">MCP tool shape</span>
                  <div
                    className={styles.profileSwitch}
                    role="group"
                    aria-labelledby="mcp-surface-label"
                  >
                    {(["compact", "native"] as ServingProfile[]).map((profile) => (
                      <button
                        type="button"
                        key={profile}
                        aria-pressed={
                          !state.compareMcp && state.profile === profile
                        }
                        aria-label={
                          profile === "compact"
                            ? "Compact shared lanes"
                            : "Native direct tool"
                        }
                        onClick={() => dispatch({ type: "profile", profile })}
                      >
                        <span>
                          {profile === "compact" ? "Compact" : "Native"}
                        </span>
                        <small>
                          {profile === "compact" ? "shared lanes" : "direct tool"}
                        </small>
                      </button>
                    ))}
                  </div>
                  <button
                    type="button"
                    className={styles.compareSurfaces}
                    aria-pressed={state.compareMcp}
                    onClick={() =>
                      dispatch({
                        type: "compareMcp",
                        active: !state.compareMcp,
                      })
                    }
                  >
                    {state.compareMcp
                      ? "Close generated comparison"
                      : "Compare both generated shapes"}
                  </button>
                  <p className={styles.displayHint}>
                    Display only · both shapes select{" "}
                    <code>{variant.mcpSurfaceComparison.operationId}</code>. The
                    request microscope remains a Native capture.
                  </p>
                </div>
              </div>
            </header>

            <fieldset className={styles.controls}>
              <legend className="srOnly">Semantic controls</legend>
              {bundle.controls.map((control) => (
                <label key={control.id}>
                  <span>{control.label}</span>
                  <select
                    disabled={state.mode === "handwritten"}
                    value={state.selection[control.id]}
                    onChange={(event) =>
                      selectValue(
                        dispatch,
                        control.id,
                        event.target.value,
                        control.factId,
                      )
                    }
                  >
                    {control.options.map((option) => (
                      <option value={option.value} key={option.value}>
                        {option.label}
                      </option>
                    ))}
                  </select>
                </label>
              ))}
            </fieldset>

            <GuidedProof
              variant={variant}
              generatorCommand={manifest.generator.command}
              mode={state.mode}
              guideStep={state.guideStep}
              activeFactId={state.activeFactId}
              profile={state.profile}
              compareMcp={state.compareMcp}
              overrides={state.overrides}
              drift={drift}
              repositorySource={repositorySource}
              onRestore={() => restoreAuthority(null)}
            />

            <div className={styles.woven} ref={wovenRef}>
              <CausalThreads
                key={`${variant.id}:${state.profile}:${state.compareMcp}`}
                containerRef={wovenRef}
                anchors={variant.semanticAnchors}
                activeFactId={activeFactId}
                mismatchedTargetIds={driftTargetIds}
              />
              <section className={styles.declaration} aria-labelledby="declaration-title">
                <header>
                  <div>
                    <p className={styles.eyebrow}>Authoritative Rust</p>
                    <h3 id="declaration-title">The command, declared once</h3>
                  </div>
                  <CopyButton value={variant.declaration.text} label="Copy Rust" />
                </header>
                <p className={styles.sourceCue} aria-live="polite">
                  <span
                    className={activeFactIsOmitted ? styles.cueHidden : ""}
                    aria-hidden={activeFactIsOmitted}
                  >
                    Hover or focus a promise to reveal the declaration lines
                    that produce it.
                  </span>
                  {omittedDeclarationFact ? (
                    <span
                      className={
                        activeFactIsOmitted ? "" : styles.cueHidden
                      }
                      aria-hidden={!activeFactIsOmitted}
                      data-code-absence={omittedDeclarationFact.id}
                      data-visible={activeFactIsOmitted}
                    >
                      <strong>{omittedDeclarationFact.label}</strong> is absent
                      by design: no declaration line is emitted in this variant.
                    </span>
                  ) : null}
                </p>
                <DeclarationCode
                  declaration={variant.declaration}
                  activeFactId={activeFactId}
                  onHoverFact={(factId) =>
                    dispatch({ type: "hoverFact", factId })
                  }
                />
                <div className={styles.factList} aria-label="Declaration facts">
                  {variant.declaration.facts.map((fact) => (
                    <button
                      type="button"
                      key={fact.id}
                      data-source-fact={fact.id}
                      data-active={activeFactId === fact.id}
                      className={
                        activeFactId === fact.id ? styles.factActive : ""
                      }
                      onFocus={() =>
                        dispatch({ type: "focusFact", factId: fact.id })
                      }
                      onPointerEnter={() =>
                        dispatch({ type: "hoverFact", factId: fact.id })
                      }
                      onPointerLeave={() =>
                        dispatch({ type: "hoverFact", factId: null })
                      }
                      onBlur={() =>
                        dispatch({ type: "focusFact", factId: null })
                      }
                    >
                      <span>{fact.id}</span>
                      <strong>{fact.label}</strong>
                      <code>{fact.displayValue}</code>
                    </button>
                  ))}
                </div>
                <details className={styles.declarationProvenance}>
                  <summary>Declaration provenance</summary>
                  <p>
                    <SourceLink path={variant.declaration.sourcePath}>
                      {variant.declaration.sourcePath}
                    </SourceLink>
                  </p>
                  <p>
                    Catalog <code>{variant.fingerprints.catalog}</code>
                  </p>
                </details>
              </section>

              <div className={styles.projectionArea}>
                <div
                  className={styles.mobileTabs}
                  role="tablist"
                  aria-label="Generated projections"
                >
                  {projections.map((projection) => (
                    <button
                      type="button"
                      role="tab"
                      key={projection}
                      data-tab={projection}
                      aria-label={
                        projection === "confirmation"
                          ? "Confirmation"
                          : undefined
                      }
                      aria-selected={state.activeProjection === projection}
                      aria-controls={`panel-${projection}`}
                      tabIndex={state.activeProjection === projection ? 0 : -1}
                      onClick={() =>
                        dispatch({ type: "projection", projection })
                      }
                      onKeyDown={(event) =>
                        handleProjectionKeys(event, projection)
                      }
                    >
                      {projection === "confirmation"
                        ? "Confirm"
                        : projection === "mcp"
                        ? "MCP"
                        : projection[0]?.toUpperCase() + projection.slice(1)}
                    </button>
                  ))}
                </div>
                <div className={styles.projections}>
                  {projections.map((projection) => (
                    <ProjectionPanel
                      key={projection}
                      projection={projection}
                      variant={variant}
                      profile={state.profile}
                      compareMcp={state.compareMcp}
                      mode={state.mode}
                      overrides={state.overrides}
                      activeFactId={activeFactId}
                      mobileActive={state.activeProjection === projection}
                      onHoverFact={(factId) =>
                        dispatch({ type: "hoverFact", factId })
                      }
                      onFocusFact={(factId) =>
                        dispatch({ type: "focusFact", factId })
                      }
                      onEditOverride={(targetId, value) =>
                        dispatch({ type: "editOverride", targetId, value })
                      }
                    />
                  ))}
                </div>
              </div>
            </div>

            <section
              className={styles.microscope}
              aria-labelledby="microscope-title"
            >
              <header>
                <div>
                  <p className={styles.eyebrow}>Request microscope</p>
                  <h3 id="microscope-title">
                    Select → bind → validate → authorize → realize → dispatch →
                    result/task
                  </h3>
                </div>
                <span>Stage taxonomy: essay-authored · native invocation capture</span>
              </header>
              <div className={styles.traceSteps}>
                {variant.trace.map((step, index) => (
                  <button
                    type="button"
                    key={step.id}
                    aria-pressed={state.traceIndex === index}
                    aria-controls="trace-detail"
                    onClick={() => dispatch({ type: "trace", index })}
                  >
                    <span>{index + 1}</span>
                    {step.label}
                  </button>
                ))}
              </div>
              <article
                id="trace-detail"
                className={styles.traceDetail}
                aria-live="polite"
              >
                <div>
                  <p className={styles.eyebrow}>
                    Step {state.traceIndex + 1} of {variant.trace.length} ·{" "}
                    {variant.trace[state.traceIndex]?.label}
                  </p>
                  <p>{variant.trace[state.traceIndex]?.summary}</p>
                </div>
                <pre tabIndex={0}>
                  <code>
                    {prettyJson(variant.trace[state.traceIndex]?.payload ?? null)}
                  </code>
                </pre>
              </article>
              <dl className={styles.identities}>
                <div>
                  <dt>Catalog</dt>
                  <dd>{shortHash(variant.fingerprints.catalog)}</dd>
                </div>
                <div>
                  <dt>Surface · native capture</dt>
                  <dd data-testid="microscope-surface">
                    {shortHash(variant.fingerprints.nativeSurface)}
                  </dd>
                </div>
                <div>
                  <dt>Invocation</dt>
                  <dd>{shortHash(variant.fingerprints.invocation)}</dd>
                </div>
              </dl>
            </section>

            <section className={styles.privateContext} aria-labelledby="private-title">
              <div>
                <p className={styles.eyebrow}>Private context</p>
                <h3 id="private-title">Useful to the decision, absent from the API</h3>
                <p>{variant.privacy.redactedSummary}</p>
              </div>
              <dl>
                <div>
                  <dt>Declared</dt>
                  <dd>{variant.privacy.declared ? "yes" : "no"}</dd>
                </div>
                <div>
                  <dt>Handler observed</dt>
                  <dd>{variant.privacy.handlerObserved ? "yes" : "not supplied"}</dd>
                </div>
                <div>
                  <dt>Raw identity serialized</dt>
                  <dd>{variant.privacy.rawIdentitySerialized ? "yes" : "no"}</dd>
                </div>
              </dl>
              <ul>
                {variant.privacy.checks.map((check) => (
                  <li key={check.surface}>
                    {check.surface}: {check.absent ? "raw identity absent" : "check failed"}
                  </li>
                ))}
              </ul>
            </section>

            <footer className={styles.bundleProvenance}>
              <p className={styles.rfcLine}>
                Design lineage:{" "}
                <SourceLink path="docs/rfcs/stage-4/0001-authoritative-command-surface.md">
                  RFC 0001 catalog authority
                </SourceLink>
                {" · "}
                <SourceLink path="docs/rfcs/stage-2/0013-conversation-identity-request-context.md">
                  RFC 0013 private identity
                </SourceLink>
                {" · "}
                <SourceLink path="docs/rfcs/stage-1/0015-catalog-derived-native-tool-surfaces.md">
                  RFC 0015 native surfaces
                </SourceLink>
                {" · "}
                <SourceLink path="docs/rfcs/stage-1/0018-declared-invocation-and-confirmation-presentation.md">
                  RFC 0018 presentation
                </SourceLink>
                .{" "}
                <SourceLink path="docs/rfcs/stage-1/0019-catalog-derived-host-adapters.md">
                  RFC 0019
                </SourceLink>{" "}
                is the separate host-adapter design; the host panel above is
                illustrative site presentation, not an implemented RFC 0019
                artifact.
              </p>
              <details>
                <summary>Catalog and evidence fingerprints</summary>
                <p>
                  Generated by <code>{manifest.generator.command}</code>. Bundle
                  format <code>1</code>; {bundle.variants.length} immutable
                  variants; {manifest.files.length} inventoried files.
                </p>
                <CopyButton
                  value={prettyJson(
                    bundle as unknown as import("./evidence/types").JsonValue,
                  )}
                  label="Copy bundle JSON"
                />
              </details>
            </footer>
            <p className="srOnly" aria-live="polite">
              {state.announcement}
            </p>
          </section>
        </section>

        <VblFieldStudy evidence={evidence} />
      </main>

      <footer className={styles.siteFooter}>
        <p>
          The browser selects generated evidence; it does not reimplement Twill
          semantics.
        </p>
      </footer>
    </>
  );
}
