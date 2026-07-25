import Ajv2020 from "ajv/dist/2020.js";
import type { ErrorObject } from "ajv";
import bundleJson from "../../public/evidence/bundle.json";
import manifestJson from "../../public/evidence/manifest.json";
import schemaJson from "../../public/evidence/schema.json";
import vblHistoricalJson from "../../public/evidence/vbl/catalog-measurement.json";
import vblFixtureJson from "../../public/evidence/vbl/v0.4.9-manifest.json";
import {
  type EvidenceBundle,
  type EvidenceManifest,
  type EvidenceVariant,
  type JsonValue,
  type LoadedEvidence,
  type ManifestFile,
  type VariantSelection,
  projections,
} from "./types";

function fail(message: string): never {
  throw new Error(`Invalid generated evidence: ${message}`);
}

function record(value: unknown, label: string): Record<string, unknown> {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    return fail(`${label} must be an object`);
  }
  return value as Record<string, unknown>;
}

function string(value: unknown, label: string): string {
  if (typeof value !== "string" || value.length === 0) {
    return fail(`${label} must be a non-empty string`);
  }
  return value;
}

function integer(value: unknown, label: string): number {
  if (!Number.isSafeInteger(value) || (value as number) < 0) {
    return fail(`${label} must be a non-negative safe integer`);
  }
  return value as number;
}

function array(value: unknown, label: string): unknown[] {
  if (!Array.isArray(value)) {
    return fail(`${label} must be an array`);
  }
  return value;
}

function formatAjvErrors(errors: ErrorObject[] | null | undefined): string {
  if (!errors?.length) return "bundle does not match schema";
  return errors
    .map(({ instancePath, message }) => `${instancePath || "/"} ${message ?? "is invalid"}`)
    .join("; ");
}

const sha256Pattern = /^[0-9a-f]{64}$/;

function selectionKey(selection: VariantSelection): string {
  return [
    selection.titleRule,
    selection.destination,
    selection.confirmation,
    selection.privateContext,
  ].join("/");
}

function assertFrontendInvariants(bundle: EvidenceBundle): void {
  const requiredHostLabel =
    "Illustrative host rendering — layout is site-owned; values are Twill-generated.";
  if (bundle.formatVersion !== 1) {
    fail(`unsupported bundle formatVersion ${String(bundle.formatVersion)}`);
  }
  if (bundle.generatedBy.name !== "cargo xtask export-site-evidence") {
    fail("unexpected generator name");
  }
  if (bundle.generatedBy.version !== 1) {
    fail("unsupported generator version");
  }
  if (bundle.generatedBy.command !== "cargo xtask export-site-evidence") {
    fail("unexpected generator command");
  }
  if (bundle.source.repository !== "wycats/mcp-twill") {
    fail("unexpected source repository");
  }
  if (!["compact", "native"].includes(bundle.defaults.profile)) {
    fail("unsupported default profile");
  }
  const expectedControlValues = new Map<string, Set<string>>([
    ["titleRule", new Set(["unconstrained", "nonEmpty"])],
    ["destination", new Set(["local", "remote"])],
    ["confirmation", new Set(["generic", "titleInterpolated"])],
    ["privateContext", new Set(["none", "conversationIdentity"])],
  ]);
  const expectedControlFacts = new Map<string, string>([
    ["titleRule", "fact.titleRule"],
    ["destination", "fact.destination"],
    ["confirmation", "fact.confirmation"],
    ["privateContext", "fact.privateContext"],
  ]);
  if (bundle.controls.length !== expectedControlValues.size) {
    fail("expected exactly four semantic controls");
  }
  for (const control of bundle.controls) {
    const expected = expectedControlValues.get(control.id);
    const actual = new Set(control.options.map((option) => option.value));
    if (
      !expected ||
      control.factId !== expectedControlFacts.get(control.id) ||
      expected.size !== actual.size ||
      [...expected].some((value) => !actual.has(value))
    ) {
      fail(`unexpected option inventory for control ${control.id}`);
    }
  }
  if (bundle.variants.length !== 16) {
    fail(`expected 16 variants, received ${bundle.variants.length}`);
  }

  const ids = new Set<string>();
  const selections = new Set<string>();
  let defaultsFound = false;

  for (const variant of bundle.variants) {
    if (ids.has(variant.id)) fail(`duplicate variant id ${variant.id}`);
    ids.add(variant.id);

    const key = selectionKey(variant.selection);
    if (selections.has(key)) fail(`duplicate variant selection ${key}`);
    selections.add(key);
    defaultsFound ||= key === selectionKey(bundle.defaults.selection);

    if (variant.trace.length !== 7) {
      fail(`${variant.id} must contain exactly seven trace steps`);
    }
    if (variant.trace.some((step) => step.authority !== "essayAuthored")) {
      fail(`${variant.id} trace authority must be essayAuthored`);
    }
    if (variant.privacy.rawIdentitySerialized !== false) {
      fail(`${variant.id} contains serialized private identity`);
    }
    if (variant.hostPreview.label !== requiredHostLabel) {
      fail(`${variant.id} has an unapproved host projection label`);
    }

    const facts = new Set(variant.declaration.facts.map((fact) => fact.id));
    for (const control of bundle.controls) {
      if (!facts.has(control.factId)) {
        fail(`${variant.id} is missing control fact ${control.factId}`);
      }
    }
    const targetIds = new Set(variant.comparisonTargets.map((target) => target.id));
    const represented = new Set(variant.comparisonTargets.map((target) => target.projection));

    for (const projection of projections) {
      if (!represented.has(projection)) {
        fail(`${variant.id} has no comparison target for ${projection}`);
      }
    }
    for (const profile of ["compact", "native"] as const) {
      if (
        !variant.comparisonTargets.some(
          (target) =>
            target.projection === "mcp" && target.profiles.includes(profile),
        )
      ) {
        fail(`${variant.id} has no MCP comparison target for ${profile}`);
      }
    }
    for (const target of variant.comparisonTargets) {
      if (!facts.has(target.factId)) {
        fail(`${variant.id} comparison target ${target.id} names missing fact ${target.factId}`);
      }
      if (
        target.profiles.length === 0 ||
        new Set(target.profiles).size !== target.profiles.length
      ) {
        fail(`${variant.id} comparison target ${target.id} has invalid profile scope`);
      }
      if (target.displayValue.length === 0) {
        fail(`${variant.id} comparison target ${target.id} has no display value`);
      }
    }
    for (const fact of variant.declaration.facts) {
      if (fact.displayValue.length === 0) {
        fail(`${variant.id} declaration fact ${fact.id} has no display value`);
      }
    }
    for (const anchor of variant.semanticAnchors) {
      if (!facts.has(anchor.sourceFact)) {
        fail(`${variant.id} semantic anchor ${anchor.id} names missing fact ${anchor.sourceFact}`);
      }
      for (const targetId of anchor.targetIds) {
        if (!targetIds.has(targetId)) {
          fail(`${variant.id} semantic anchor ${anchor.id} names missing target ${targetId}`);
        }
      }
    }
  }

  if (!defaultsFound) fail("default selection has no generated variant");
  if (selections.size !== 16) fail("variant matrix is incomplete");
}

function normalizeManifest(value: unknown): EvidenceManifest {
  const raw = record(value, "manifest");
  if (raw.formatVersion !== 1) {
    fail(`unsupported manifest formatVersion ${String(raw.formatVersion)}`);
  }

  const filesValue = raw.files;
  const files: ManifestFile[] = Array.isArray(filesValue)
    ? filesValue.map((item, index) => {
        const file = record(item, `manifest.files[${index}]`);
        return {
          path: string(file.path, `manifest.files[${index}].path`),
          sha256: string(file.sha256, `manifest.files[${index}].sha256`),
          bytes: integer(file.bytes, `manifest.files[${index}].bytes`),
        };
      })
    : Object.entries(record(filesValue, "manifest.files")).map(([path, item]) => {
        const file = record(item, `manifest.files.${path}`);
        return {
          path,
          sha256: string(file.sha256, `manifest.files.${path}.sha256`),
          bytes: integer(file.bytes, `manifest.files.${path}.bytes`),
        };
      });

  if (!files.some(({ path }) => path.endsWith("bundle.json"))) {
    fail("manifest does not inventory bundle.json");
  }
  if (!files.some(({ path }) => path.endsWith("schema.json"))) {
    fail("manifest does not inventory schema.json");
  }
  if (new Set(files.map(({ path }) => path)).size !== files.length) {
    fail("manifest contains duplicate file paths");
  }
  for (const file of files) {
    if (!sha256Pattern.test(file.sha256)) {
      fail(`manifest file ${file.path} has an invalid SHA-256`);
    }
    if (file.bytes === 0) {
      fail(`manifest file ${file.path} must not be empty`);
    }
  }

  return {
    formatVersion: 1,
    generator: (() => {
      const generator = record(raw.generator, "manifest.generator");
      return {
        name: string(generator.name, "manifest.generator.name"),
        version: integer(generator.version, "manifest.generator.version"),
        command: string(generator.command, "manifest.generator.command"),
      };
    })(),
    sources: array(raw.sources, "manifest.sources").map((item, index) => {
      const source = record(item, `manifest.sources[${index}]`);
      return {
        path: string(source.path, `manifest.sources[${index}].path`),
        sha256: string(source.sha256, `manifest.sources[${index}].sha256`),
        provenance: string(
          source.provenance,
          `manifest.sources[${index}].provenance`,
        ),
      };
    }),
    files,
  };
}

function assertProvenanceAgreement(
  bundle: EvidenceBundle,
  manifest: EvidenceManifest,
): void {
  const sourcePaths = bundle.source.paths;
  const sourceHashPaths = Object.keys(bundle.source.sourceHashes);
  if (sourcePaths.length === 0) {
    fail("bundle source inventory must not be empty");
  }
  if (
    new Set(sourcePaths).size !== sourcePaths.length ||
    sourceHashPaths.length !== sourcePaths.length ||
    sourcePaths.some((path) => !sourceHashPaths.includes(path))
  ) {
    fail("bundle source paths and sourceHashes must have exact matching keys");
  }
  for (const path of sourcePaths) {
    if (!sha256Pattern.test(bundle.source.sourceHashes[path] ?? "")) {
      fail(`bundle source ${path} has an invalid SHA-256`);
    }
  }

  if (
    manifest.generator.name !== bundle.generatedBy.name ||
    manifest.generator.version !== bundle.generatedBy.version ||
    manifest.generator.command !== bundle.generatedBy.command
  ) {
    fail("manifest generator does not match bundle generator");
  }

  if (
    manifest.sources.length !== sourcePaths.length ||
    new Set(manifest.sources.map(({ path }) => path)).size !==
      manifest.sources.length
  ) {
    fail("manifest source inventory does not exactly match bundle sources");
  }
  const manifestSources = new Map(
    manifest.sources.map((source) => [source.path, source] as const),
  );
  for (const path of sourcePaths) {
    const source = manifestSources.get(path);
    if (!source || source.sha256 !== bundle.source.sourceHashes[path]) {
      fail(`manifest source ${path} does not match bundle provenance`);
    }
    if (!sha256Pattern.test(source.sha256) || source.provenance.trim() === "") {
      fail(`manifest source ${path} has incomplete provenance`);
    }
  }

  const expectedEvidenceFiles = new Set([
    "bundle.json",
    "schema.json",
    "vbl/catalog-measurement.json",
    "vbl/v0.4.9-manifest.json",
  ]);
  const actualEvidenceFiles = new Set(manifest.files.map(({ path }) => path));
  if (
    actualEvidenceFiles.size !== expectedEvidenceFiles.size ||
    [...expectedEvidenceFiles].some((path) => !actualEvidenceFiles.has(path))
  ) {
    fail("manifest evidence file inventory is not exact");
  }
}

export function parseEvidence(
  bundleValue: unknown,
  schemaValue: unknown,
  manifestValue: unknown,
  vblValue: {
    historicalMeasurement: JsonValue;
    frozenFixtureManifest: JsonValue;
  } = {
    historicalMeasurement: null,
    frozenFixtureManifest: null,
  },
): LoadedEvidence {
  const ajv = new Ajv2020({ allErrors: true, strict: false });
  ajv.addFormat("uint32", {
    type: "number",
    validate: (value: number) =>
      Number.isInteger(value) && value >= 0 && value <= 4_294_967_295,
  });
  const validate = ajv.compile(record(schemaValue, "schema"));
  if (!validate(bundleValue)) {
    fail(formatAjvErrors(validate.errors));
  }

  const bundle = bundleValue as EvidenceBundle;
  assertFrontendInvariants(bundle);

  const manifest = normalizeManifest(manifestValue);
  assertProvenanceAgreement(bundle, manifest);

  return {
    bundle,
    manifest,
    vbl: vblValue,
  };
}

export function loadTrackedEvidence(): LoadedEvidence {
  return parseEvidence(bundleJson, schemaJson, manifestJson, {
    historicalMeasurement: vblHistoricalJson as JsonValue,
    frozenFixtureManifest: vblFixtureJson as JsonValue,
  });
}

export function findVariant(
  variants: EvidenceVariant[],
  selection: VariantSelection,
): EvidenceVariant {
  const key = selectionKey(selection);
  const match = variants.find((variant) => selectionKey(variant.selection) === key);
  return match ?? fail(`no generated variant for ${key}`);
}

export function prettyJson(value: JsonValue): string {
  return JSON.stringify(value, null, 2);
}
