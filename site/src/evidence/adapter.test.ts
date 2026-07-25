import bundleJson from "../../public/evidence/bundle.json";
import manifestJson from "../../public/evidence/manifest.json";
import schemaJson from "../../public/evidence/schema.json";
import {
  findVariant,
  loadTrackedEvidence,
  parseEvidence,
} from "./adapter";
import type { VariantSelection } from "./types";

describe("generated evidence adapter", () => {
  it("accepts the tracked canonical bundle and complete variant matrix", () => {
    const { bundle, manifest } = loadTrackedEvidence();
    expect(bundle.formatVersion).toBe(1);
    expect(bundle.variants).toHaveLength(16);
    expect(manifest.files.some(({ path }) => path.endsWith("bundle.json"))).toBe(
      true,
    );
  });

  it("maps every control combination to exactly one generated variant", () => {
    const { bundle } = loadTrackedEvidence();
    const selections: VariantSelection[] = [];
    for (const titleRule of ["unconstrained", "nonEmpty"] as const) {
      for (const destination of ["local", "remote"] as const) {
        for (const confirmation of ["generic", "titleInterpolated"] as const) {
          for (const privateContext of [
            "none",
            "conversationIdentity",
          ] as const) {
            selections.push({
              titleRule,
              destination,
              confirmation,
              privateContext,
            });
          }
        }
      }
    }

    expect(
      new Set(
        selections.map(
          (selection) => findVariant(bundle.variants, selection).id,
        ),
      ).size,
    ).toBe(16);
  });

  it("fails closed on an unknown bundle version", () => {
    const invalid = structuredClone(bundleJson) as Record<string, unknown>;
    invalid.formatVersion = 2;
    expect(() => parseEvidence(invalid, schemaJson, manifestJson)).toThrow(
      /Invalid generated evidence/,
    );
  });

  it("fails closed on a missing variant", () => {
    const invalid = structuredClone(bundleJson);
    invalid.variants.pop();
    expect(() => parseEvidence(invalid, schemaJson, manifestJson)).toThrow(
      /16 variants|schema/,
    );
  });

  it("fails closed on incomplete provenance", () => {
    const invalid = structuredClone(manifestJson) as Record<string, unknown>;
    invalid.files = [];
    expect(() => parseEvidence(bundleJson, schemaJson, invalid)).toThrow(
      /bundle\.json/,
    );
  });

  it("fails closed when a source hash disagrees with the manifest", () => {
    const invalid = structuredClone(bundleJson);
    const path = invalid.source.paths[0]!;
    const sourceHashes = invalid.source.sourceHashes as Record<string, string>;
    sourceHashes[path] = "f".repeat(64);
    expect(() => parseEvidence(invalid, schemaJson, manifestJson)).toThrow(
      /does not match bundle provenance/,
    );
  });

  it("fails closed when source provenance is removed", () => {
    const invalid = structuredClone(manifestJson);
    invalid.sources[0]!.provenance = "";
    expect(() => parseEvidence(bundleJson, schemaJson, invalid)).toThrow(
      /provenance/,
    );
  });

  it("fails closed when generator identities disagree", () => {
    const invalid = structuredClone(manifestJson);
    invalid.generator.command = "other generator";
    expect(() => parseEvidence(bundleJson, schemaJson, invalid)).toThrow(
      /generator/,
    );
  });

  it("fails closed when a generated comparison target is removed", () => {
    const invalid = structuredClone(bundleJson) as unknown as {
      variants: Array<{
        comparisonTargets: Array<{ projection: string }>;
      }>;
    };
    invalid.variants[0]!.comparisonTargets =
      invalid.variants[0]!.comparisonTargets.filter(
        (target) => target.projection !== "host",
      );
    expect(() => parseEvidence(invalid, schemaJson, manifestJson)).toThrow(
      /host/,
    );
  });
});
