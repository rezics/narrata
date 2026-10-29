// The public authoring journey of the install test. It is copied into the consumer project and
// imports nothing but its arguments: Node, Bun and a browser Worker pass the installed entries.
const decoder = new TextDecoder();
const encoder = new TextEncoder();
function check(condition, message) { if (!condition) throw new Error(message); }

function snapshot(publication, lock) {
  return {
    id: publication.publicationId, artifact: publication.artifactId, names: publication.description.names,
    description: publication.descriptionBytes, lock,
    objects: new Map([...publication.objects()].map(([reference, bytes]) => [reference.object_id, bytes])),
  };
}

/** Builds the short story from public types, previews it, publishes A, edits it and publishes B. */
export async function author(api, { shortStorySource, shortStoryOutline }) {
  const source = shortStorySource();
  const minter = api.createMinter();
  const filled = minter.fill(source.manifest, source.packages);
  minter.free();
  check(filled.diagnostics.length === 0 && filled.minted.length === 11, "minting the short story failed");
  const bad = api.validateRecord(encoder.encode('{"format_version":1,"kind":"graph","package":"story"}'));
  check(!bad.record && bad.diagnostics.some(d => d.severity === "error"), "a bad record was accepted");

  const drafts = api.createDrafts();
  for (const record of filled.records) drafts.add(record.bytes);
  drafts.addOutline(shortStoryOutline());
  const preview = drafts.preview();
  check(preview.compiled, "preview failed");
  const previewed = { manifest: preview.manifest(), names: preview.names(), objects: new Map(preview.report.objects.map(id => [id, preview.object(id)])) };
  preview.free();
  const first = drafts.publish();
  drafts.free();
  check(first.report.status === "switchable" && first.publication, "publication A failed");
  const a = snapshot(first.publication, first.report.lock);
  first.publication.free();

  // Remove an option, then publish B against A restored from its description and single objects.
  const edited = first.records.map(record => {
    const draft = JSON.parse(decoder.decode(record.bytes));
    if (draft.kind === "choice_point" && draft.payload.key === "return") draft.payload.options = draft.payload.options.filter(option => option.key !== "wander");
    return draft;
  });
  const previous = await api.readPublication(a.description, a.id, async reference => a.objects.get(reference.object_id));
  const next = api.createDrafts();
  for (const record of edited) next.add(record);
  next.addOutline(shortStoryOutline());
  const locked = next.publish({ previous, lock: a.lock });
  check(!locked.publication && locked.report.diagnostics.some(d => d.code === "tombstones_needed"), "locked publication appended tombstones");
  const second = next.publish({ previous });
  next.free();
  previous.free();
  check(second.publication && second.report.compared && second.report.previous_artifact_id === a.artifact, "publication B lost continuity");
  check(second.report.tombstones.length === 1, "the removed option has no tombstone");
  const b = snapshot(second.publication, second.report.lock);
  second.publication.free();
  return { previewed, a, b, minted: filled.minted.length };
}

async function choose(book, key) {
  const view = (await book.inspect()).view;
  check(view.interaction.kind === "choose", "expected a choice");
  const option = view.interaction.options.find(option => option.key === key);
  check(option, `missing option ${key}`);
  await book.choose(view.cursor, view.interaction.choice_point, [option.id]);
}

/** Reads the preview, then journeys on A and B from on-demand objects. */
export async function read(openBook, execution, { previewed, a, b }) {
  const previewBook = await openBook({ manifest: previewed.manifest, names: previewed.names, execution, fetchChunk: async id => previewed.objects.get(id) });
  await choose(previewBook, "forest");
  await previewBook.close();
  const open = (publication, requests = []) => openBook({
    manifest: publication.objects.get(publication.artifact.slice("artifact:".length)), names: publication.objects.get(publication.names), execution,
    fetchChunk: async id => { requests.push(id); return publication.objects.get(id.slice("object:".length)); },
  });
  const requests = [];
  const journey = await open(a, requests);
  const firstScreen = requests.length;
  await choose(journey, "forest");
  const save = await journey.exportSave();
  const cursor = (await journey.inspect()).view.cursor;
  await journey.close();
  const resumed = await open(a);
  await resumed.restore(save);
  check((await resumed.inspect()).view.cursor === cursor, "the old journey did not resume on A");
  await resumed.close();
  const fresh = await open(b);
  let bound = false;
  try { await fresh.restore(save); } catch { bound = true; }
  check(bound, "an A save restored onto B");
  await choose(fresh, "forest");
  const view = (await fresh.inspect()).view;
  check(view.interaction.kind === "choose" && view.interaction.options.map(option => option.key).join() === "home", "B does not contain the edit");
  await fresh.close();
  return { firstScreen, artifacts: [a.artifact, b.artifact], objects: [a.objects.size, b.objects.size] };
}
