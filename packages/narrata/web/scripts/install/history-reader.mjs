import { openBook } from "@rezics/narrata/reader";

function same(actual, expected, label) {
  if (JSON.stringify(actual) !== JSON.stringify(expected)) throw new Error(label);
}
/** Provided-name evidence through the installed public reader, with no historical full packs. */
export async function historyReaderJourney(input, bytes) {
  const metadata = [], chunks = [];
  const source = async artifact => {
    const entry = input.artifacts.find(entry => entry.artifact === artifact);
    if (!entry) throw new Error("Unknown exact artifact");
    metadata.push(artifact);
    return { manifest: await bytes(entry.manifest), names: await bytes(entry.names), fetchChunk: async id => {
      chunks.push({ artifact, object: id });
      const path = entry.objects[id]; if (!path) throw new Error("Wrong artifact chunk loader");
      return bytes(path);
    } };
  };
  const current = await source(input.current);
  const book = await openBook({ ...current, execution: input.execution, loadArtifact: source });
  const checked = [];
  try {
    await book.restore(input.checkpoint);
    for (const step of input.steps) {
      await book.checkout(step.commit);
      const before = await book.inspect();
      same(before.view, step.before.view, "Historical identity, timeline or names differ");
      same(before.page, step.before.page, "Historical page differs");
      if (before.view.history.some(row => !row.key)) throw new Error("A provided historical alias was lost");
      same(await book.nextContentUnits(), step.lookahead, "Historical lookahead differs");
      if (step.choice) {
        await book.choose(step.commit, step.choice.point, step.choice.options);
        const after = await book.inspect();
        same(after.view, step.after.view, "Historical continuation differs");
        same(after.page, step.after.page, "Historical continuation page differs");
        same(await book.exportSave(), step.save, "Historical save differs");
      }
      checked.push({ artifact: before.view.artifact_id, cursor: before.view.cursor, keys: before.view.history.map(row => row.key) });
    }
    if (new Set(metadata).size !== 3 || new Set(checked.map(row => row.artifact)).size !== 3) throw new Error("The three provided-name artifacts were not exercised");
  } finally { await book.close(); }
  const incomingChecks = [];
  for (const incoming of input.incoming) {
    // Only C is initially registered on this new handle. Incoming A itself must
    // trigger exact dispatch; earlier historical operations cannot pre-register it.
    const fresh = await openBook({ ...current, execution: input.execution, loadArtifact: source });
    try {
      await fresh.restore(incoming.save);
      const actual = await fresh.inspect();
      same(actual.view, incoming.view, `${incoming.format} root dispatch or names differ`);
      same(actual.page, incoming.page, `${incoming.format} root page differs`);
      same(await fresh.nextContentUnits(), incoming.lookahead, `${incoming.format} root lookahead differs`);
      incomingChecks.push({ format: incoming.format, artifact: actual.view.artifact_id, keys: actual.view.frames.map(frame => frame.key) });
    } finally { await fresh.close(); }
  }
  return { metadata, chunks, checked, incoming: incomingChecks };
}
