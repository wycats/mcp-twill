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
  initialState,
  mismatches,
  seedHandwrittenDrift,
  workbenchReducer,
} from "./state";
import { CausalThreads } from "./components/CausalThreads";
import { CopyButton } from "./components/CopyButton";
import { ProjectionPanel } from "./components/ProjectionPanel";
import styles from "./App.module.css";

const repositorySource =
  "https://github.com/wycats/mcp-twill/blob/main/";

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
}: {
  number: string;
  title: string;
  children: React.ReactNode;
  action: (trigger: HTMLButtonElement) => void;
  actionLabel?: string;
}) {
  return (
    <section className={styles.guideStep}>
      <p className={styles.eyebrow}>{number}</p>
      <h2>{title}</h2>
      <p>{children}</p>
      <button type="button" onClick={(event) => action(event.currentTarget)}>
        {actionLabel}
      </button>
    </section>
  );
}

function SourceLink({ path, children }: { path: string; children: React.ReactNode }) {
  return (
    <a href={`${repositorySource}${path}`}>
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
          <SourceLink path="docs/adoption/visible-browser-lab/baseline/catalog-measurement.json">
            Inspect historical source
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
          <SourceLink path="crates/mcp-twill/tests/fixtures/vbl/v0.4.9/manifest.json">
            Inspect frozen fixture
          </SourceLink>
          <details className={styles.fieldEvidence}>
            <summary>Frozen evidence copy</summary>
            <CopyButton
              value={prettyJson(evidence.vbl.frozenFixtureManifest)}
              label="Copy fixture JSON"
            />
            <pre tabIndex={0}>
              <code>{prettyJson(evidence.vbl.frozenFixtureManifest)}</code>
            </pre>
          </details>
        </article>
      </div>
      <p className={styles.ownershipNote}>
        Ownership is enforced in{" "}
        <SourceLink path="crates/mcp-twill/tests/support/vbl.rs">
          the VBL test support
        </SourceLink>
        . The older gap map is not used as current implementation authority.
      </p>
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
  ) {
    if (state.mode === "handwritten") dispatch({ type: "restore" });
    dispatch({ type: "replaceSelection", selection, factId });
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
              title="One command, many surfaces"
              action={() => showSelection(defaults, null)}
            >
              Read one <code>issues create</code> declaration across five
              synchronized projections.
            </GuideStep>
            <GuideStep
              number="02"
              title="A constraint travels"
              action={() => showSelection(withTitleLimit, titleFactId)}
            >
              Require a non-empty title. The same <code>minLength: 1</code>{" "}
              fact reaches every interpretation that needs it.
            </GuideStep>
            <GuideStep
              number="03"
              title="Behavior travels too"
              action={() => showSelection(withRemote, destinationFactId)}
            >
              Move from local write to remote write plus network. Permissions,
              authorization, annotations, and host warning move together.
            </GuideStep>
            <GuideStep
              number="04"
              title="Drift is the bug"
              action={enterHandwritten}
            >
              Let independently maintained surfaces disagree. The check can
              name the divergence, but cannot choose which lie to trust.
            </GuideStep>
            <GuideStep
              number="05"
              title="Restore authority"
              action={restoreAuthority}
              actionLabel="Restore from catalog"
            >
              Clear handwritten overrides atomically, then inspect the same
              request from selection through result.
            </GuideStep>
            <GuideStep
              number="06"
              title="Private context stays private"
              action={() => showSelection(withIdentity, privateContextFactId)}
            >
              A host-supplied conversation identity reaches the handler without
              becoming a public argument or result.
            </GuideStep>
            <GuideStep
              number="07"
              title="Same semantics, another surface"
              action={() => {
                dispatch({ type: "profile", profile: "compact" });
                dispatch({ type: "projection", projection: "mcp" });
              }}
            >
              Compare the compact effect lane with the native operation tool.
              Their shapes differ; their catalog authority does not.
            </GuideStep>
          </nav>

          <section className={styles.workbench} data-testid="workbench">
            <header className={styles.workbenchHeader}>
              <div>
                <p className={styles.eyebrow}>Persistent specimen</p>
                <h2 id="workbench-title">
                  <code>issues create</code>
                </h2>
              </div>
              <div className={styles.authoritySwitch} aria-label="Authority mode">
                <button
                  ref={derivedButtonRef}
                  type="button"
                  aria-pressed={state.mode === "derived"}
                  onClick={(event) => restoreAuthority(event.currentTarget)}
                >
                  Derived {state.mode === "derived" ? "●" : "○"}
                </button>
                <button
                  ref={handwrittenButtonRef}
                  type="button"
                  aria-pressed={state.mode === "handwritten"}
                  onClick={enterHandwritten}
                >
                  Handwritten {state.mode === "handwritten" ? "●" : "○"}
                </button>
              </div>
            </header>

            <div className={styles.controls}>
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
              <fieldset className={styles.profileSwitch}>
                <legend>MCP profile</legend>
                <div>
                  {(["compact", "native"] as ServingProfile[]).map((profile) => (
                    <button
                      type="button"
                      key={profile}
                      aria-pressed={state.profile === profile}
                      onClick={() => dispatch({ type: "profile", profile })}
                    >
                      {profile === "compact" ? "Compact" : "Native"}
                    </button>
                  ))}
                </div>
              </fieldset>
            </div>

            <div
              className={`${styles.agreement} ${
                drift.length ? styles.agreementFailed : styles.agreementPassed
              }`}
              role="status"
            >
              <strong>Workbench agreement check:</strong>{" "}
              {drift.length
                ? `${drift.length} mismatches found.`
                : "all evidence-declared comparison targets agree."}
              {drift.length ? (
                <ul>
                  {drift.map((target) => (
                    <li key={target.id}>
                      {target.projection} · {target.label}: expected{" "}
                      <code>{target.displayValue}</code>, received{" "}
                      <code>{state.overrides[target.id]}</code>
                    </li>
                  ))}
                </ul>
              ) : null}
              {state.mode === "handwritten" ? (
                <button
                  type="button"
                  onClick={() => restoreAuthority(null)}
                >
                  Restore from catalog
                </button>
              ) : null}
            </div>

            <div className={styles.woven} ref={wovenRef}>
              <CausalThreads
                key={`${variant.id}:${state.profile}`}
                containerRef={wovenRef}
                anchors={variant.semanticAnchors}
                activeFactId={state.activeFactId}
                mismatchedTargetIds={driftTargetIds}
              />
              <section className={styles.declaration} aria-labelledby="declaration-title">
                <header>
                  <div>
                    <p className={styles.eyebrow}>Authoritative input</p>
                    <h3 id="declaration-title">Rust declaration</h3>
                  </div>
                  <CopyButton value={variant.declaration.text} label="Copy Rust" />
                </header>
                <pre tabIndex={0}>
                  <code>{variant.declaration.text}</code>
                </pre>
                <div className={styles.factList} aria-label="Declaration facts">
                  {variant.declaration.facts.map((fact) => (
                    <button
                      type="button"
                      key={fact.id}
                      data-source-fact={fact.id}
                      className={
                        state.activeFactId === fact.id ? styles.factActive : ""
                      }
                      onFocus={() =>
                        dispatch({ type: "activateFact", factId: fact.id })
                      }
                      onPointerEnter={() =>
                        dispatch({ type: "activateFact", factId: fact.id })
                      }
                      onPointerLeave={() =>
                        dispatch({ type: "activateFact", factId: null })
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
                      {projection === "mcp"
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
                      mode={state.mode}
                      overrides={state.overrides}
                      activeFactId={state.activeFactId}
                      mobileActive={state.activeProjection === projection}
                      onActivateFact={(factId) =>
                        dispatch({ type: "activateFact", factId })
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
                    onClick={() => dispatch({ type: "trace", index })}
                  >
                    <span>{index + 1}</span>
                    {step.label}
                  </button>
                ))}
              </div>
              <article className={styles.traceDetail}>
                <div>
                  <p className={styles.eyebrow}>
                    {variant.trace[state.traceIndex]?.stage}
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
