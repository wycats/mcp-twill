import { loadTrackedEvidence, findVariant } from "./evidence/adapter";
import {
  initialState,
  mismatches,
  seedHandwrittenDrift,
  workbenchReducer,
} from "./state";

describe("workbench reducer", () => {
  const { bundle } = loadTrackedEvidence();
  const variant = findVariant(bundle.variants, bundle.defaults.selection);

  it("moves guided and free exploration through the same generated selection", () => {
    const initial = initialState(
      bundle.defaults.selection,
      bundle.defaults.profile,
    );
    const next = workbenchReducer(initial, {
      type: "select",
      control: "titleRule",
      value: "nonEmpty",
      factId: "fact.titleRule",
    });
    expect(next.selection.titleRule).toBe("nonEmpty");
    expect(next.activeFactId).toBe("fact.titleRule");
    expect(
      findVariant(bundle.variants, next.selection).selection.titleRule,
    ).toBe("nonEmpty");
  });

  it("keeps generated evidence immutable while handwritten leaves drift", () => {
    const canonical = structuredClone(variant.comparisonTargets);
    const overrides = seedHandwrittenDrift(variant);
    const handwritten = workbenchReducer(
      initialState(bundle.defaults.selection, bundle.defaults.profile),
      { type: "enterHandwritten", overrides, factId: "fact.titleRule" },
    );

    expect(mismatches(variant, handwritten.overrides)).toHaveLength(3);
    expect(variant.comparisonTargets).toEqual(canonical);
    expect(Object.isFrozen(handwritten.overrides)).toBe(true);
  });

  it("disables semantic state changes until authority is restored", () => {
    const handwritten = workbenchReducer(
      initialState(bundle.defaults.selection, bundle.defaults.profile),
      {
        type: "enterHandwritten",
        overrides: seedHandwrittenDrift(variant),
        factId: "fact.titleRule",
      },
    );
    const unchanged = workbenchReducer(handwritten, {
      type: "select",
      control: "destination",
      value: "remote",
      factId: "fact.destination",
    });
    expect(unchanged).toBe(handwritten);

    const restored = workbenchReducer(handwritten, { type: "restore" });
    expect(restored.mode).toBe("derived");
    expect(restored.overrides).toEqual({});
    expect(restored.announcement).toMatch(/passes/);
  });

  it("switches profiles, mobile projection tabs, and trace steps independently", () => {
    const initial = initialState(
      bundle.defaults.selection,
      bundle.defaults.profile,
    );
    const compact = workbenchReducer(initial, {
      type: "profile",
      profile: "compact",
    });
    const schema = workbenchReducer(compact, {
      type: "projection",
      projection: "schema",
    });
    const trace = workbenchReducer(schema, { type: "trace", index: 4 });
    expect(trace.profile).toBe("compact");
    expect(trace.activeProjection).toBe("schema");
    expect(trace.traceIndex).toBe(4);
  });
});
