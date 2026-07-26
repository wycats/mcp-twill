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

  it("fails closed when MCP surface comparison evidence is missing", () => {
    const invalid = structuredClone(bundleJson);
    delete (
      invalid.variants[0] as unknown as Record<string, unknown>
    ).mcpSurfaceComparison;
    expect(() => parseEvidence(invalid, schemaJson, manifestJson)).toThrow(
      /mcpSurfaceComparison|schema/,
    );
  });

  it("fails closed when MCP comparison facts disagree", () => {
    const unpublished = structuredClone(bundleJson);
    unpublished.variants[0]!.mcpSurfaceComparison.compact.toolName =
      "not-published";
    expect(() =>
      parseEvidence(unpublished, schemaJson, manifestJson),
    ).toThrow(/unpublished tool/);

    const unknownInput = structuredClone(bundleJson);
    unknownInput.variants[0]!.mcpSurfaceComparison.native.requiredInputs.push(
      "missing",
    );
    expect(() =>
      parseEvidence(unknownInput, schemaJson, manifestJson),
    ).toThrow(/unknown input/);

    const wrongOperation = structuredClone(bundleJson);
    wrongOperation.variants[0]!.mcpSurfaceComparison.operationId =
      "other.operation";
    expect(() =>
      parseEvidence(wrongOperation, schemaJson, manifestJson),
    ).toThrow(/wrong catalog operation/);
  });

  it("fails closed when declaration code ranges are missing", () => {
    const invalid = structuredClone(bundleJson);
    const fact = invalid.variants[0]!.declaration
      .facts[0] as unknown as Record<string, unknown>;
    delete fact.codeRanges;
    expect(() => parseEvidence(invalid, schemaJson, manifestJson)).toThrow(
      /codeRanges|schema/,
    );
  });

  it("fails closed on reversed or out-of-bounds declaration code ranges", () => {
    const reversed = structuredClone(bundleJson);
    reversed.variants[0]!.declaration.facts[0]!.codeRanges[0] = {
      startLine: 9,
      endLine: 4,
    };
    expect(() => parseEvidence(reversed, schemaJson, manifestJson)).toThrow(
      /invalid code range/,
    );

    const outOfBounds = structuredClone(bundleJson);
    outOfBounds.variants[0]!.declaration.facts[0]!.codeRanges[0] = {
      startLine: 1,
      endLine: 10_000,
    };
    expect(() => parseEvidence(outOfBounds, schemaJson, manifestJson)).toThrow(
      /invalid code range/,
    );
  });

  it("fails closed when declaration facts claim the same code line", () => {
    const invalid = structuredClone(bundleJson);
    const titleRange =
      invalid.variants[0]!.declaration.facts.find(
        (fact) => fact.id === "fact.titleRule",
      )!.codeRanges[0]!;
    invalid.variants[0]!.declaration.facts.find(
      (fact) => fact.id === "fact.destination",
    )!.codeRanges = [{ ...titleRange }];
    expect(() => parseEvidence(invalid, schemaJson, manifestJson)).toThrow(
      /belongs to/,
    );
  });

  it("fails closed on duplicate declaration facts", () => {
    const invalid = structuredClone(bundleJson);
    const facts = invalid.variants[0]!.declaration
      .facts as unknown as Array<Record<string, unknown>>;
    const privateFact = facts.find(
      (fact) => fact.id === "fact.privateContext",
    )!;
    facts.push(structuredClone(privateFact));
    expect(() => parseEvidence(invalid, schemaJson, manifestJson)).toThrow(
      /duplicate declaration fact ids/,
    );
  });

  it("fails closed when generated code presence disagrees with its ranges", () => {
    const invalid = structuredClone(bundleJson);
    invalid.variants[0]!.declaration.facts[0]!.codePresence = "omitted";
    expect(() => parseEvidence(invalid, schemaJson, manifestJson)).toThrow(
      /code presence disagrees/,
    );
  });
});
