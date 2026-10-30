import { openBook, locateSave } from "@rezics/narrata";
import { IndexedDbStore, decodeLoaded, encodeLoadRequest, WriteRejected } from "@rezics/narrata/storage";

function check(condition, message) { if (!condition) throw new Error(message); }
const same = (a, b) => JSON.stringify(a) === JSON.stringify(b);
const exactPage = ({view,page}) => ({ page, cursor: view.cursor, presentation: view.presentation, shared: view.shared, frames: view.frames, interaction: view.interaction });
const execution = "execution:0190f2a0000070008000000000000001";
const empty = encodeLoadRequest({ keys: [], objects: [], keyRanges: [], objectRanges: [] });
async function choose(book) {
  const view = (await book.inspect()).view;
  if (view.interaction.kind !== "choose") return false;
  await book.choose(view.cursor, view.interaction.choice_point, [view.interaction.options.find(o => o.enabled).id]);
  return true;
}
async function roots(handle) {
  const result = await handle.run({ kind: "roots", after: null, limit: 1024 });
  check(result.kind === "roots" && !result.more, "Incomplete roots");
  return result.items;
}

/** The same unpacked public-entry journey runs in Node, Bun and Chromium. */
export async function verifyMaintenance(api, pack) {
  const source = await IndexedDbStore.open(`maintenance-source-${crypto.randomUUID()}`);
  const target = await IndexedDbStore.open(`maintenance-target-${crypto.randomUUID()}`);
  const publication = api.publicationFromPack(pack);
  let loads = 0, replyBytes = 0, writes = 0, quota = false;
  const create = async store => {
    const state = decodeLoaded(await store.load(empty)).store;
    const handle = api.createMaintenance({storeId:state.id, storage:{
      load: async bytes => { loads++; const reply = await store.load(bytes); replyBytes += reply.length; return reply; },
      persist: bytes => { writes++; return quota ? Promise.reject(new WriteRejected("quota")) : store.persist(bytes); },
    }});
    await handle.registerPublication(publication);
    return handle;
  };
  const book = await openBook({pack,execution,storage:source});
  const exporting = await create(source), importing = await create(target);
  try {
    await choose(book); await choose(book);
    const snapshot = await book.inspect(), before = snapshot.view;
    const selection = {execution,artifact:book.artifactId,cursor:before.cursor};
    const sizes = [];
    for (const ancestors of [undefined, 0, 1]) {
      const bytes = await exporting.exportSave({...selection, ...(ancestors === undefined ? {} : {ancestors})});
      const checked = await importing.inspectSave(bytes);
      check(checked.cursor === selection.cursor && checked.artifact === selection.artifact, "Exact save identity");
      const observed = await locateSave(target,execution) ? (await roots(importing)).find(r=>r.key.namespace === 3) : null;
      const selected = {...selection, expected_revision:observed?.revision ?? null, now:100};
      if (ancestors === undefined) {
        quota = true;
        try { await importing.importSave(bytes,selected); throw new Error("Quota accepted"); }
        catch (error) { check(error instanceof WriteRejected && error.reason === "quota", "Definite quota rejection"); }
        quota = false;
        check(await locateSave(target,execution) === null, "Quota changed the durable root");
      }
      await importing.importSave(bytes,selected);
      const reopened = await openBook({pack,execution,storage:target});
      const reference = await openBook({pack,execution});
      try {
        await reference.restore(await book.exportSave());
        check(same(exactPage(await reopened.inspect()),exactPage(snapshot)), "Shallow page/args/state/interaction differ");
        await choose(reopened); await choose(reference);
        check(same(exactPage(await reopened.inspect()),exactPage(await reference.inspect())), "Next choice differs after cold import");
      } finally { await reopened.close(); await reference.close(); }
      sizes.push(bytes.length);
    }
    const selected = await roots(importing);
    await importing.run({kind:"delete_roots",roots:selected});
    try { await importing.run({kind:"delete_roots",roots:selected}); throw new Error("Stale root deletion accepted"); }
    catch (error) { check(error.code === "stale_input", "Root CAS did not reject stale observation"); }
    const revision = decodeLoaded(await target.load(empty)).store.revision;
    const dry = await importing.run({kind:"collect",now:1000,grace_seconds:10,dry_run:true});
    check(decodeLoaded(await target.load(empty)).store.revision === revision, "Dry-run wrote data");
    const collected = await importing.run({kind:"collect",now:1000,grace_seconds:10,dry_run:false});
    check(same(dry.removed,collected.removed), "Dry-run and complete sweep differ");
    check(same(await importing.run({kind:"integrity"}),{kind:"integrity",issues:[]}), "Integrity after GC");
    check(await locateSave(target,execution) === null, "Deleted root reappeared");
    const bytes = await exporting.exportSave({...selection,ancestors:0});
    await importing.importSave(bytes,{...selection,expected_revision:null,now:1001});
    check((await locateSave(target,execution))?.cursor === before.cursor, "Cleanup/retry did not restore the exact cursor");
    check(loads > 0 && writes > 0, "Journey did not use cold host transport");
    return {sizes,loads,replyBytes,writes,rootCas:true,dryRunReadOnly:true,quotaRollback:true,coldContinue:true};
  } finally {
    await Promise.all([book.close(),exporting.close(),importing.close()]);
    publication.free(); source.close(); target.close();
  }
}
