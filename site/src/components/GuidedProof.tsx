import type {
  ComparisonTarget,
  EvidenceVariant,
  ProjectionName,
  ServingProfile,
} from "../evidence/types";
import { projections } from "../evidence/types";
import type { AuthorityMode, GuideStepId } from "../state";
import styles from "./GuidedProof.module.css";

const projectionPromises: Record<
  ProjectionName,
  { audience: string; promise: string }
> = {
  help: { audience: "People", promise: "Help" },
  schema: { audience: "Runtime", promise: "Schema" },
  mcp: { audience: "Agents", promise: "MCP tool" },
  confirmation: { audience: "Approval UI", promise: "Confirmation" },
  host: { audience: "Application UI", promise: "Host rendering" },
};

interface GuidedProofProps {
  variant: EvidenceVariant;
  generatorCommand: string;
  mode: AuthorityMode;
  guideStep: GuideStepId | null;
  activeFactId: string | null;
  profile: ServingProfile;
  compareMcp: boolean;
  overrides: Readonly<Record<string, string>>;
  drift: ComparisonTarget[];
  repositorySource: string;
  onRestore: () => void;
}

function targetInventory(
  variant: EvidenceVariant,
  factId: string | null,
  profile: ServingProfile,
): ComparisonTarget[] {
  if (!factId) return [];
  const anchor = variant.semanticAnchors.find(
    (candidate) => candidate.sourceFact === factId,
  );
  if (!anchor) return [];
  const targets = new Map(
    variant.comparisonTargets.map((target) => [target.id, target] as const),
  );

  return anchor.targetIds.flatMap((targetId) => {
    const target = targets.get(targetId);
    return target && target.profiles.includes(profile) ? [target] : [];
  });
}

function projectionLabel(projection: ProjectionName): string {
  return projection === "confirmation"
    ? "Confirmation presentation"
    : projection === "mcp"
      ? "MCP"
      : projection[0]!.toUpperCase() + projection.slice(1);
}

function FactProof({
  variant,
  factId,
  profile,
  guideStep,
}: {
  variant: EvidenceVariant;
  factId: string;
  profile: ServingProfile;
  guideStep: GuideStepId | null;
}) {
  const fact = variant.declaration.facts.find(
    (candidate) => candidate.id === factId,
  );
  const targets = targetInventory(variant, factId, profile);
  const authorization = variant.trace.find(
    (step) => step.stage === "authorize",
  );

  if (!fact) return null;

  return (
    <>
      <div className={styles.causalProof}>
        <div className={styles.sourceFact} data-proof-fact={fact.id}>
          <span>Generated source fact</span>
          <strong>{fact.label}</strong>
          <code>{fact.displayValue}</code>
        </div>
        <div className={styles.targetGroup}>
          <span>Generated consequences</span>
          <ul>
            {targets.map((target) => (
              <li key={target.id} data-proof-target={target.id}>
                <span>{projectionLabel(target.projection)}</span>
                <strong>{target.label}</strong>
                <code>{target.displayValue}</code>
              </li>
            ))}
          </ul>
        </div>
      </div>
      {guideStep === 3 && authorization ? (
        <p className={styles.explanation}>
          <span>Request consequence · essay-authored trace</span>
          {authorization.summary}
        </p>
      ) : null}
      {guideStep === 6 ? (
        <dl className={styles.privacyProof} aria-label="Generated privacy proof">
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
      ) : null}
    </>
  );
}

function DriftProof({
  drift,
  overrides,
  onRestore,
}: {
  drift: ComparisonTarget[];
  overrides: Readonly<Record<string, string>>;
  onRestore: () => void;
}) {
  return (
    <div className={styles.driftProof}>
      <div>
        <span>Workbench exercise</span>
        <strong>
          {drift.length
            ? "Independent copies now contradict their generated source."
            : "The copies match again, but each still owns an editable truth."}
        </strong>
      </div>
      <ul>
        {drift.map((target) => (
          <li key={target.id} data-proof-target={target.id}>
            <span>{projectionLabel(target.projection)}</span>
            <code>{overrides[target.id]}</code>
            <small>
              Catalog: <code>{target.displayValue}</code>
            </small>
          </li>
        ))}
      </ul>
      <button type="button" onClick={onRestore}>
        Restore from catalog
      </button>
    </div>
  );
}

function EnforcementProof({
  generatorCommand,
  repositorySource,
  catalogFingerprint,
}: {
  generatorCommand: string;
  repositorySource: string;
  catalogFingerprint: string;
}) {
  return (
    <>
      <div className={styles.enforcementProof}>
        <article>
          <span>1 · Generate real surfaces</span>
          <strong>Rust captures both serving profiles and the request.</strong>
          <a
            href={`${repositorySource}crates/mcp-twill/examples/issues_server/site_specimen.rs`}
          >
            Inspect the specimen
          </a>
        </article>
        <article>
          <span>2 · Reject inconsistency</span>
          <strong>
            Catalog, surface, causal-inventory, and privacy checks must pass.
          </strong>
          <a href={`${repositorySource}xtask/src/site_evidence.rs`}>
            Inspect the generator gates
          </a>
        </article>
        <article>
          <span>3 · Block stale evidence</span>
          <strong>
            <code>{generatorCommand} --check</code> regenerates and byte-compares.
          </strong>
          <a href={`${repositorySource}.github/workflows/ci.yml`}>
            Inspect the CI gate
          </a>
        </article>
      </div>
      <p className={styles.boundaryNote}>
        Twill consumers use catalog-derived runtime projections directly. Snapshot
        comparison is this essay’s additional build gate. Catalog{" "}
        <code>{catalogFingerprint}</code>
      </p>
    </>
  );
}

function McpComparisonProof({ variant }: { variant: EvidenceVariant }) {
  const comparison = variant.mcpSurfaceComparison;
  return (
    <>
      <div className={styles.mcpProof}>
        <div>
          <span>Catalog operation</span>
          <strong>
            <code>{comparison.operationId}</code>
          </strong>
        </div>
        <article>
          <span>Compact · shared tools</span>
          <strong>{comparison.compact.toolName}</strong>
          <code>
            {comparison.compact.requiredInputs.join(" + ")}
            {comparison.compact.hasArgumentMap ? " + args map" : ""}
          </code>
        </article>
        <article>
          <span>Native · direct tool</span>
          <strong>{comparison.native.toolName}</strong>
          <code>{comparison.native.requiredInputs.join(" + ")}</code>
        </article>
      </div>
      <p className={styles.explanation}>
        <span>Why it matters · essay explanation</span>
        Compact shares a small tool vocabulary across commands; Native publishes
        one operation-specific tool. Effect lanes are shared execution and
        authorization buckets selected from declared behavior.
      </p>
    </>
  );
}

export function GuidedProof({
  variant,
  generatorCommand,
  mode,
  guideStep,
  activeFactId,
  profile,
  compareMcp,
  overrides,
  drift,
  repositorySource,
  onRestore,
}: GuidedProofProps) {
  const hasFactProof = Boolean(activeFactId) && mode === "derived";
  const headline =
    mode === "handwritten"
      ? drift.length === 0
        ? "Handwritten values match—but authority remains split."
        : `${drift.length} handwritten ${
            drift.length === 1 ? "promise contradicts" : "promises contradict"
          } the catalog.`
      : compareMcp
        ? "One catalog operation. Two public call shapes."
        : guideStep === 5
          ? "Authority restored—and checked before this site ships."
          : hasFactProof
            ? "One source fact. Every affected promise."
            : guideStep === 1
              ? "One declaration. Five synchronized projections."
              : "Choose a chapter to stage its evidence.";

  return (
    <section
      id="guided-proof"
      className={styles.proof}
      aria-label="Guided proof"
      data-testid="guided-proof"
      data-guide-step={guideStep ?? "none"}
    >
      <header className={styles.header}>
        <div>
          <p>
            Guided proof
            {guideStep ? ` · ${String(guideStep).padStart(2, "0")}` : ""}
          </p>
          <h3 id="guided-proof-title">{headline}</h3>
        </div>
        <p
          className={drift.length ? styles.failed : styles.passed}
          role="status"
          aria-live="polite"
          aria-atomic="true"
        >
          <strong>Workbench agreement check:</strong>{" "}
          {drift.length
            ? `${drift.length} mismatches found.`
            : "all generated comparison targets agree."}
        </p>
      </header>

      {mode === "handwritten" ? (
        <DriftProof drift={drift} overrides={overrides} onRestore={onRestore} />
      ) : compareMcp ? (
        <McpComparisonProof variant={variant} />
      ) : guideStep === 5 ? (
        <EnforcementProof
          generatorCommand={generatorCommand}
          repositorySource={repositorySource}
          catalogFingerprint={variant.fingerprints.catalog}
        />
      ) : hasFactProof && activeFactId ? (
        <FactProof
          variant={variant}
          factId={activeFactId}
          profile={profile}
          guideStep={guideStep}
        />
      ) : guideStep === 1 ? (
        <div className={styles.promiseProof}>
          <div>
            <span>Authoritative source</span>
            <strong>Rust command declaration</strong>
            <code>{variant.mcpSurfaceComparison.operationId}</code>
          </div>
          <ol>
            {projections.map((projection) => (
              <li key={projection} data-proof-projection={projection}>
                <span>{projectionPromises[projection].audience}</span>
                <strong>{projectionPromises[projection].promise}</strong>
              </li>
            ))}
          </ol>
        </div>
      ) : (
        <p className={styles.prompt}>
          The selected source fact and its Rust-generated consequences will stay
          together here while you move through the essay.
        </p>
      )}
    </section>
  );
}
