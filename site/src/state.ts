import type {
  ComparisonTarget,
  EvidenceVariant,
  ProjectionName,
  ServingProfile,
  VariantSelection,
} from "./evidence/types";

export type AuthorityMode = "derived" | "handwritten";

export interface WorkbenchState {
  selection: VariantSelection;
  profile: ServingProfile;
  mode: AuthorityMode;
  overrides: Readonly<Record<string, string>>;
  activeFactId: string | null;
  activeProjection: ProjectionName;
  traceIndex: number;
  announcement: string;
}

type SelectAction = {
  [Control in keyof VariantSelection]: {
    type: "select";
    control: Control;
    value: VariantSelection[Control];
    factId: string;
  };
}[keyof VariantSelection];

export type WorkbenchAction =
  | SelectAction
  | {
      type: "replaceSelection";
      selection: VariantSelection;
      factId: string | null;
    }
  | { type: "profile"; profile: ServingProfile }
  | {
      type: "enterHandwritten";
      overrides: Record<string, string>;
      factId: string;
    }
  | { type: "editOverride"; targetId: string; value: string }
  | { type: "restore" }
  | { type: "activateFact"; factId: string | null }
  | { type: "projection"; projection: ProjectionName }
  | { type: "trace"; index: number };

export function initialState(
  selection: VariantSelection,
  profile: ServingProfile,
): WorkbenchState {
  return {
    selection,
    profile,
    mode: "derived",
    overrides: {},
    activeFactId: null,
    activeProjection: "help",
    traceIndex: 0,
    announcement: "Catalog-derived projections are in agreement.",
  };
}

export function workbenchReducer(
  state: WorkbenchState,
  action: WorkbenchAction,
): WorkbenchState {
  switch (action.type) {
    case "select":
      if (state.mode === "handwritten") return state;
      return {
        ...state,
        selection: {
          ...state.selection,
          [action.control]: action.value,
        } as VariantSelection,
        activeFactId: action.factId,
        announcement: `Selected generated ${action.control} variant.`,
      };
    case "replaceSelection":
      if (state.mode === "handwritten") return state;
      return {
        ...state,
        selection: action.selection,
        activeFactId: action.factId,
        announcement: "Guided state loaded from generated evidence.",
      };
    case "profile":
      return {
        ...state,
        profile: action.profile,
        activeProjection: "mcp",
        announcement: `${action.profile === "compact" ? "Compact" : "Native"} MCP projection selected.`,
      };
    case "enterHandwritten":
      return {
        ...state,
        mode: "handwritten",
        overrides: Object.freeze({ ...action.overrides }),
        activeProjection: "help",
        activeFactId: action.factId,
        announcement: "Handwritten drift introduced. Agreement check failed.",
      };
    case "editOverride":
      if (state.mode !== "handwritten") return state;
      return {
        ...state,
        overrides: Object.freeze({
          ...state.overrides,
          [action.targetId]: action.value,
        }),
        announcement: "Handwritten projection updated.",
      };
    case "restore":
      return {
        ...state,
        mode: "derived",
        overrides: {},
        activeFactId: null,
        announcement:
          "Restored from catalog. Workbench agreement check passes.",
      };
    case "activateFact":
      return { ...state, activeFactId: action.factId };
    case "projection":
      return { ...state, activeProjection: action.projection };
    case "trace":
      return {
        ...state,
        traceIndex: action.index,
        announcement: `Request microscope step ${action.index + 1} selected.`,
      };
  }
}

export function seedHandwrittenDrift(
  variant: EvidenceVariant,
): Record<string, string> {
  const overrides = Object.fromEntries(
    variant.comparisonTargets
      .filter((target) => target.editable)
      .map((target) => [target.id, target.displayValue]),
  );
  const replacements: Partial<Record<ComparisonTarget["projection"], string>> = {
    help: "minLength 5",
    schema: "minLength 2",
    host: "No minimum",
  };

  for (const projection of ["help", "schema", "host"] as const) {
    const target = variant.comparisonTargets.find(
      (candidate) => candidate.editable && candidate.projection === projection,
    );
    if (target) {
      overrides[target.id] =
        replacements[projection] ?? target.displayValue;
    }
  }

  return overrides;
}

export function mismatches(
  variant: EvidenceVariant,
  overrides: Readonly<Record<string, string>>,
): ComparisonTarget[] {
  return variant.comparisonTargets.filter(
    (target) =>
      target.editable &&
      overrides[target.id] !== undefined &&
      overrides[target.id] !== target.displayValue,
  );
}
