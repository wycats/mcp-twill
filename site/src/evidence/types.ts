export type JsonPrimitive = string | number | boolean | null;
export type JsonValue =
  | JsonPrimitive
  | JsonValue[]
  | { [key: string]: JsonValue };

export type TitleRule = "unconstrained" | "nonEmpty";
export type Destination = "local" | "remote";
export type ConfirmationKind = "generic" | "titleInterpolated";
export type PrivateContext = "none" | "conversationIdentity";
export type ServingProfile = "compact" | "native";
export type ProjectionName =
  | "help"
  | "schema"
  | "mcp"
  | "confirmation"
  | "host";

export interface VariantSelection {
  titleRule: TitleRule;
  destination: Destination;
  confirmation: ConfirmationKind;
  privateContext: PrivateContext;
}

export interface GeneratedFact {
  id: string;
  label: string;
  value: JsonValue;
  displayValue: string;
  targetIds: string[];
}

export interface DeclarationEvidence {
  sourcePath: string;
  text: string;
  facts: GeneratedFact[];
}

export interface HostArgumentField {
  name: string;
  label: string;
  required: boolean;
  constraint: string;
}

export interface HostPreview {
  label: string;
  title: string;
  description: string;
  argumentFields: HostArgumentField[];
  effectBadges: string[];
  confirmation: {
    title: string;
    message: string;
  };
  taskSupport: string;
  privateContext: {
    declared: boolean;
    label: string;
  };
}

export type TraceStage =
  | "select"
  | "bind"
  | "validate"
  | "authorize"
  | "realize"
  | "dispatch"
  | "resultTask";

export interface TraceStep {
  id: string;
  stage: TraceStage;
  label: string;
  authority: "essayAuthored";
  summary: string;
  payload: JsonValue;
}

export interface SemanticAnchor {
  id: string;
  label: string;
  sourceFact: string;
  targetIds: string[];
}

export interface ComparisonTarget {
  id: string;
  factId: string;
  projection: ProjectionName;
  label: string;
  value: JsonValue;
  displayValue: string;
  editable: boolean;
  profiles: ServingProfile[];
}

export interface PrivacyEvidence {
  declared: boolean;
  handlerObserved: boolean;
  rawIdentitySerialized: false;
  redactedSummary: string;
  checks: Array<{
    surface: string;
    absent: boolean;
  }>;
}

export interface Fingerprints {
  catalog: string;
  runSchema: string;
  helpSchema: string;
  compactSurface: string;
  nativeSurface: string;
  invocation: string;
}

export interface EvidenceVariant {
  id: string;
  selection: VariantSelection;
  declaration: DeclarationEvidence;
  rustConfiguration: JsonValue;
  catalogOperation: JsonValue;
  help: JsonValue;
  argumentSchema: JsonValue;
  compact: {
    tools: JsonValue;
    selectedTool: JsonValue;
    surfaceIdentity: JsonValue;
  };
  native: {
    surface: JsonValue;
    tool: JsonValue;
    surfaceIdentity: JsonValue;
  };
  confirmation: JsonValue;
  hostPreview: HostPreview;
  plan: JsonValue;
  trace: TraceStep[];
  result: JsonValue;
  fingerprints: Fingerprints;
  semanticAnchors: SemanticAnchor[];
  comparisonTargets: ComparisonTarget[];
  privacy: PrivacyEvidence;
}

export interface EvidenceBundle {
  formatVersion: 1;
  generatedBy: {
    name: "cargo xtask export-site-evidence";
    version: 1;
    command: "cargo xtask export-site-evidence";
  };
  source: {
    repository: "wycats/mcp-twill";
    paths: string[];
    sourceHashes: Record<string, string>;
  };
  defaults: {
    selection: VariantSelection;
    profile: ServingProfile;
  };
  controls: Array<{
    id: keyof VariantSelection;
    factId: string;
    label: string;
    options: Array<{
      value: string;
      label: string;
    }>;
  }>;
  variants: EvidenceVariant[];
}

export interface ManifestFile {
  path: string;
  sha256: string;
  bytes: number;
}

export interface EvidenceManifest {
  formatVersion: 1;
  generator: {
    name: string;
    version: number;
    command: string;
  };
  sources: Array<{
    path: string;
    sha256: string;
    provenance: string;
  }>;
  files: ManifestFile[];
}

export interface LoadedEvidence {
  bundle: EvidenceBundle;
  manifest: EvidenceManifest;
  vbl: {
    historicalMeasurement: JsonValue;
    frozenFixtureManifestRaw: string;
  };
}

export const projections: ProjectionName[] = [
  "help",
  "schema",
  "mcp",
  "confirmation",
  "host",
];
