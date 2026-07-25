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
  mode: AuthorityMode;
  overrides: Readonly<Record<string, string>>;
  activeFactId: string | null;
  mobileActive: boolean;
  onActivateFact: (factId: string | null) => void;
  onEditOverride: (targetId: string, value: string) => void;
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
  onActivate,
  onEdit,
}: {
  target: ComparisonTarget;
  mode: AuthorityMode;
  value: string;
  active: boolean;
  onActivate: (factId: string | null) => void;
  onEdit: (targetId: string, value: string) => void;
}) {
  const drifted = value !== target.displayValue;
  return (
    <div
      className={`${styles.target} ${active ? styles.active : ""} ${drifted ? styles.drifted : ""}`}
      data-comparison-target={target.id}
      data-fact-id={target.factId}
      onPointerEnter={() => onActivate(target.factId)}
      onPointerLeave={() => onActivate(null)}
    >
      <span className={styles.factId}>{target.factId}</span>
      <label htmlFor={`target-${target.id}`}>{target.label}</label>
      {mode === "handwritten" && target.editable ? (
        <input
          id={`target-${target.id}`}
          value={value}
          onChange={(event) => onEdit(target.id, event.target.value)}
          onFocus={() => onActivate(target.factId)}
        />
      ) : (
        <button
          id={`target-${target.id}`}
          type="button"
          className={styles.generatedValue}
          onFocus={() => onActivate(target.factId)}
          onClick={() => onActivate(target.factId)}
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
  mode,
  overrides,
  activeFactId,
  mobileActive,
  onActivateFact,
  onEditOverride,
}: ProjectionPanelProps) {
  const targets = variant.comparisonTargets.filter(
    (target) =>
      target.projection === projection && target.profiles.includes(profile),
  );
  const raw = rawProjection(projection, variant, profile);
  const rawText = prettyJson(raw);
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
      role="tabpanel"
    >
      <header className={styles.header}>
        <h3 id={`projection-${projection}`}>
          {projectionLabels[projection]}
        </h3>
        <span>
          {projection === "mcp" ? `${profile} · ` : ""}
          {mode === "derived" ? "catalog-derived" : "handwritten"}
        </span>
      </header>

      {projection === "host" ? <HostPreviewView variant={variant} /> : null}

      <div className={styles.targets}>
        {targets.map((target) => (
          <TargetRow
            key={target.id}
            target={target}
            mode={mode}
            value={overrides[target.id] ?? target.displayValue}
            active={activeFactId === target.factId}
            onActivate={onActivateFact}
            onEdit={onEditOverride}
          />
        ))}
      </div>

      <details className={styles.details}>
        <summary>Raw generated {projectionLabels[projection]}</summary>
        <CopyButton value={rawText} label="Copy JSON" />
        <pre tabIndex={0}>
          <code>{rawText}</code>
        </pre>
      </details>
      <details className={styles.details}>
        <summary>Provenance</summary>
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
          <div>
            <dt>Surface</dt>
            <dd>
              <code>{surfaceFingerprint}</code>
            </dd>
          </div>
          <div>
            <dt>Facts</dt>
            <dd>{targets.map((target) => target.factId).join(", ")}</dd>
          </div>
        </dl>
      </details>
    </section>
  );
}
