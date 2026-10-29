import type { AssessmentV1, Authoring, EvolutionController, PlanV1, Publication } from "@rezics/narrata/authoring";

export interface EvolutionVector {
  name: string;
  source: number[];
  target: number[];
  save: string;
  declarations?: string;
  descriptor?: number[];
  expected: AssessmentV1;
  applied?: unknown;
  targetSave?: string;
  ancestor?: number[];
}
function ensure(value: unknown, message: string): asserts value { if (!value) throw new Error(message); }
function sorted(value: unknown): unknown {
  if (Array.isArray(value)) return value.map(sorted);
  if (value && typeof value === "object") return Object.fromEntries(Object.entries(value).sort(([a],[b]) => a.localeCompare(b)).map(([key,value]) => [key,sorted(value)]));
  return value;
}
function equal(a: unknown, b: unknown, label: string): void { ensure(JSON.stringify(sorted(a)) === JSON.stringify(sorted(b)), label); }

/** Runs entirely through public installed-package calls in Node, Bun and Chromium. */
export async function evolutionJourney(api: Authoring, vector: EvolutionVector, Controller?: typeof EvolutionController): Promise<string> {
  const previous = api.publicationFromPack(Uint8Array.from(vector.source));
  let candidate: Publication | undefined = api.publicationFromPack(Uint8Array.from(vector.target));
  const plan = api.planEvolution(previous,candidate,vector.declarations);
  const evolution = api.createEvolution();
  const owned: Publication[] = [previous];
  try {
    const ids: string[] = [];
    if (plan.report.kind === "descriptor") {
      equal([...plan.bytes() ?? []],vector.descriptor,`${vector.name}: descriptor bytes`);
      const checked = api.readEvolution(plan.bytes() ?? new Uint8Array(),plan.report.migration_id,previous,candidate);
      try { equal(checked.report,plan.report,"descriptor roundtrip"); } finally { checked.free(); }
      candidate = candidate.withEvolution(previous,plan);
      ids.push(await evolution.registerEvolution(previous,candidate));
      // An installed host persists/reloads one object at a time, with source validation.
      const objects = new Map([...candidate.objects()].map(([r,b]) => [r.object_id,b]));
      const reloaded = await api.readPublication(candidate.descriptionBytes,candidate.publicationId, async r => {
        const bytes = objects.get(r.object_id); ensure(bytes,"missing publication object"); return bytes;
      },previous);
      equal(reloaded.publicationId,candidate.publicationId,"publication readback identity");
      reloaded.free();
    } else {
      ensure(plan.bytes() === undefined,"Exact cannot create a self edge");
      await evolution.registerPublication(previous);
      await evolution.registerPublication(candidate);
    }
    if (vector.ancestor) {
      const ancestor = api.publicationFromPack(Uint8Array.from(vector.ancestor)); owned.push(ancestor);
      let intermediate = api.publicationFromPack(Uint8Array.from(vector.source));
      const ancestorPlan = api.planEvolution(ancestor,intermediate);
      try { intermediate = intermediate.withEvolution(ancestor,ancestorPlan); owned.push(intermediate); await evolution.registerEvolution(ancestor,intermediate); }
      finally { ancestorPlan.free(); }
    }
    await evolution.restoreSave(vector.save);
    const options = { target: candidate.artifactId, migrationIds: ids };
    const controller = Controller && new Controller(evolution);
    const assessment = await (controller ? controller.assess(options) : evolution.assess(options));
    // A restored snapshot has its own cursor revision. Every semantic/report/commit byte
    // remains fixed by the Rust corpus, while revisions belong to the current store.
    const expected = structuredClone(vector.expected);
    if ((assessment.kind === "automatic" || assessment.kind === "needs_confirmation") && (expected.kind === "automatic" || expected.kind === "needs_confirmation")) expected.plan.expected_cursor_revision = assessment.plan.expected_cursor_revision;
    equal(assessment,expected,`${vector.name}: assessment`);
    if (assessment.kind === "automatic" || assessment.kind === "needs_confirmation") {
      const approved: PlanV1 = assessment.plan;
      if (controller && assessment.kind === "needs_confirmation") {
        controller.continueOld();
        ensure(controller.state.kind === "continued_old","continue old remains explicit");
        await controller.assess(options);
      }
      const result = controller ? await (assessment.kind === "needs_confirmation" ? controller.confirm() : controller.applyAutomatic()) : await evolution.apply(approved);
      equal(result,vector.applied,`${vector.name}: apply`);
      if (vector.targetSave) equal(await (vector.name === "shallow-history" ? evolution.exportShallow(1) : evolution.exportSave()),vector.targetSave,`${vector.name}: checkpoint bytes`);
    }
    return assessment.kind;
  } finally {
    await evolution.close(); plan.free(); candidate?.free(); for (const p of owned) p.free();
  }
}
