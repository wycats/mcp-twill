import type {
  ComparisonTarget,
  EvidenceVariant,
  JsonValue,
  ProjectionName,
  ServingProfile,
} from "../evidence/types";
import { prettyJson } from "../evidence/adapter";
import type { AuthorityMode } from "../state";
import { CopyButton } from "./CopyButton";
import styles from "./ProjectionPanel.module.css";

const projectionLabels: Record<ProjectionName, string> = {
  help: "Help",
  schema: "Schema",
  mcp: "MCP",
  confirmation: "Confirmation",
  host: "Host",
};

interface ProjectionPanelProps {
  projection: ProjectionName;
  variant: EvidenceVariant;
  profile: ServingProfile;
  compareMcp: boolean;
  mode: AuthorityMode;
  overrides: Readonly<Record<string, string>>;
  activeFactId: string | null;
  mobileActive: boolean;
  onHoverFact: (factId: string | null) => void;
  onFocusFact: (factId: string | null) => void;
  onEditOverride: (targetId: string, value: string) => void;
}

function joined(values: string[]): string {
  return values.length ? values.join(" · ") : "none";
}

function McpSurfaceComparisonView({
  variant,
}: {
  variant: EvidenceVariant;
}) {
  const comparison = variant.mcpSurfaceComparison;

  return (
    <div
      className={styles.surfaceComparison}
      aria-label="Compact and Native MCP tool comparison"
    >
      <p className={styles.comparisonIntro}>
        <span>Essay explanation</span>
        <strong>
          “Compact” describes how the tool surface scales across many commands,
          not which side has fewer tools in this one-command snapshot.
        </strong>
      </p>
      <div className={styles.generatedComparison}>
        <p className={styles.comparisonLabel}>Generated evidence</p>
        <div className={styles.surfaceMatrix}>
          <div
            className={`${styles.matrixHeading} ${styles.compactHeading}`}
            data-surface-shape="compact"
          >
            <span>Compact</span>
            <strong>Shared effect lanes</strong>
          </div>
          <div
            className={`${styles.matrixHeading} ${styles.nativeHeading}`}
            data-surface-shape="native"
          >
            <span>Native</span>
            <strong>Direct operation tool</strong>
          </div>

          <p className={styles.matrixLabel}>Tool used here</p>
          <code className={styles.matrixValue}>
            {comparison.compact.toolName}
          </code>
          <code className={styles.matrixValue}>
            {comparison.native.toolName}
          </code>

          <p className={styles.matrixLabel}>Argument carrier</p>
          <span className={styles.matrixValue}>
            <code>{joined(comparison.compact.requiredInputs)}</code>
            {comparison.compact.hasArgumentMap ? " + args map" : ""}
          </span>
          <span className={styles.matrixValue}>
            <code>{joined(comparison.native.requiredInputs)}</code>
            {comparison.native.hasArgumentMap ? " + args map" : ""}
          </span>

          <p className={styles.matrixLabel}>Published inventory</p>
          <code className={styles.matrixValue}>
            {joined(comparison.compact.toolInventory)}
          </code>
          <code className={styles.matrixValue}>
            {joined(comparison.native.toolInventory)}
          </code>
        </div>
      </div>
      <p className={styles.comparisonInvariant}>
        Different published shapes. Both select catalog operation{" "}
        <code>{comparison.operationId}</code>.
      </p>
    </div>
  );
}

function rawProjection(
  projection: ProjectionName,
  variant: EvidenceVariant,
  profile: ServingProfile,
): JsonValue {
  switch (projection) {
    case "help":
      return variant.help;
    case "schema":
      return variant.argumentSchema;
    case "mcp":
      return profile === "compact"
        ? variant.compact.selectedTool
        : variant.native.tool;
    case "confirmation":
      return variant.confirmation;
    case "host":
      return variant.hostPreview as unknown as JsonValue;
  }
}

function TargetRow({
  target,
  mode,
  value,
  active,
  onHover,
  onFocus,
  onEdit,
}: {
  target: ComparisonTarget;
  mode: AuthorityMode;
  value: string;
  active: boolean;
  onHover: (factId: string | null) => void;
  onFocus: (factId: string | null) => void;
  onEdit: (targetId: string, value: string) => void;
}) {
  const drifted = value !== target.displayValue;
  return (
    <div
      className={`${styles.target} ${active ? styles.active : ""} ${drifted ? styles.drifted : ""}`}
      data-comparison-target={target.id}
      data-fact-id={target.factId}
      onPointerEnter={() => onHover(target.factId)}
      onPointerLeave={() => onHover(null)}
      onBlur={(event) => {
        if (!event.currentTarget.contains(event.relatedTarget)) {
          onFocus(null);
        }
      }}
    >
      <span className={styles.factId}>{target.factId}</span>
      <label htmlFor={`target-${target.id}`}>{target.label}</label>
      {mode === "handwritten" && target.editable ? (
        <input
          id={`target-${target.id}`}
          value={value}
          onChange={(event) => onEdit(target.id, event.target.value)}
          onFocus={() => onFocus(target.factId)}
        />
      ) : (
        <button
          id={`target-${target.id}`}
          type="button"
          className={styles.generatedValue}
          onFocus={() => onFocus(target.factId)}
        >
          {value}
        </button>
      )}
      <span className={styles.origin}>
        Origin: declaration fact <code>{target.factId}</code>
      </span>
    </div>
  );
}

function HostPreviewView({ variant }: { variant: EvidenceVariant }) {
  const host = variant.hostPreview;
  return (
    <div className={styles.hostPreview}>
      <p className={styles.hostLabel}>{host.label}</p>
      <h4>{host.title}</h4>
      <p>{host.description}</p>
      <dl>
        {host.argumentFields.map((field) => (
          <div key={field.name}>
            <dt>
              {field.label} {field.required ? "(required)" : "(optional)"}
            </dt>
            <dd>{field.constraint}</dd>
          </div>
        ))}
      </dl>
      <p>
        {host.effectBadges.map((badge) => (
          <span className={styles.effectBadge} key={badge}>
            {badge}
          </span>
        ))}
      </p>
      <p>
        <strong>{host.confirmation.title}</strong>
        <br />
        {host.confirmation.message}
      </p>
      <p className={styles.hostMeta}>
        Tasks: {host.taskSupport}. {host.privateContext.label}
      </p>
    </div>
  );
}

export function ProjectionPanel({
  projection,
  variant,
  profile,
  compareMcp,
  mode,
  overrides,
  activeFactId,
  mobileActive,
  onHoverFact,
  onFocusFact,
  onEditOverride,
}: ProjectionPanelProps) {
  const comparingSurfaces = projection === "mcp" && compareMcp;
  const targets = variant.comparisonTargets.filter(
    (target) =>
      target.projection === projection &&
      (comparingSurfaces || target.profiles.includes(profile)),
  );
  const raw = rawProjection(projection, variant, profile);
  const rawEvidence = comparingSurfaces
    ? [
        {
          label: `Compact · ${variant.mcpSurfaceComparison.compact.toolName}`,
          value: variant.compact.selectedTool,
        },
        {
          label: `Native · ${variant.mcpSurfaceComparison.native.toolName}`,
          value: variant.native.tool,
        },
      ]
    : [{ label: `Raw generated ${projectionLabels[projection]}`, value: raw }];
  const surfaceFingerprint =
    projection === "mcp"
      ? profile === "compact"
        ? variant.fingerprints.compactSurface
        : variant.fingerprints.nativeSurface
      : variant.fingerprints.catalog;

  return (
    <section
      id={`panel-${projection}`}
      className={`${styles.panel} ${mobileActive ? styles.mobileActive : ""}`}
      aria-labelledby={`projection-${projection}`}
      data-projection-panel={projection}
      data-mcp-view={
        projection === "mcp"
          ? comparingSurfaces
            ? "comparison"
            : profile
          : undefined
      }
      role="tabpanel"
    >
      <header className={styles.header}>
        <h3 id={`projection-${projection}`}>
          {projectionLabels[projection]}
        </h3>
        {projection === "mcp" ? (
          <span>
            {comparingSurfaces
              ? "shared lanes ↔ direct tool"
              : profile === "compact"
                ? "compact · shared effect lanes"
                : "native · direct operation tool"}{" "}
            · {mode === "derived" ? "catalog-derived" : "handwritten"}
          </span>
        ) : null}
      </header>

      {projection === "host" ? <HostPreviewView variant={variant} /> : null}

      {comparingSurfaces ? (
        <McpSurfaceComparisonView variant={variant} />
      ) : (
        <div className={styles.targets}>
          {targets.map((target) => (
            <TargetRow
              key={target.id}
              target={target}
              mode={mode}
              value={overrides[target.id] ?? target.displayValue}
              active={activeFactId === target.factId}
              onHover={onHoverFact}
              onFocus={onFocusFact}
              onEdit={onEditOverride}
            />
          ))}
        </div>
      )}

      <details className={styles.details}>
        <summary>Evidence &amp; provenance</summary>
        {rawEvidence.map((item) => {
          const rawText = prettyJson(item.value);
          return (
            <div className={styles.rawEvidence} key={item.label}>
              <p>{item.label}</p>
              <CopyButton value={rawText} label={`Copy ${item.label} JSON`} />
              <pre tabIndex={0}>
                <code>{rawText}</code>
              </pre>
            </div>
          );
        })}
        <dl className={styles.provenance}>
          <div>
            <dt>Source</dt>
            <dd>
              <a
                href={`https://github.com/wycats/mcp-twill/blob/main/${variant.declaration.sourcePath}`}
              >
                {variant.declaration.sourcePath}
              </a>
            </dd>
          </div>
          <div>
            <dt>Catalog</dt>
            <dd>
              <code>{variant.fingerprints.catalog}</code>
            </dd>
          </div>
          {comparingSurfaces ? (
            <>
              <div>
                <dt>Compact</dt>
                <dd>
                  <code>{variant.fingerprints.compactSurface}</code>
                </dd>
              </div>
              <div>
                <dt>Native</dt>
                <dd>
                  <code>{variant.fingerprints.nativeSurface}</code>
                </dd>
              </div>
            </>
          ) : (
            <div>
              <dt>Surface</dt>
              <dd>
                <code>{surfaceFingerprint}</code>
              </dd>
            </div>
          )}
          <div>
            <dt>Facts</dt>
            <dd>{targets.map((target) => target.factId).join(", ")}</dd>
          </div>
        </dl>
      </details>
    </section>
  );
}
