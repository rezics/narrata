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
  const selected = view.interaction.options.filter(o => o.enabled).slice(0,Math.max(1,view.interaction.min)).map(o=>o.id);
  if (!selected.length && view.interaction.min > 0) return false;
  await book.choose(view.cursor, view.interaction.choice_point, selected);
  return true;
}

export async function verifyHistoricalMaintenance(api, load) {
  const results = [];
  for (const name of ["unseen-rule","republished-after-migration","proposal-retained","shallow-history","legacy"]) {
    const packs = [];
    for (const file of name === "legacy" ? ["story.narpack"] : ["source.narpack","target.narpack",...(name === "republished-after-migration" ? ["ancestor.narpack"] : [])]) {
      const bytes = await load(`${name}/${file}`);
      if (bytes) packs.push(bytes);
    }
    const publications = packs.map(bytes=>api.publicationFromPack(bytes));
    const stored = await IndexedDbStore.open(`maintenance-history-${crypto.randomUUID()}`);
    const state = decodeLoaded(await stored.load(empty)).store;
    const m = api.createMaintenance({storage:stored,storeId:state.id});
    let reference, continued;
    let cold;
    let receiving;
    try {
      for (const publication of publications) await m.registerPublication(publication);
      const input = await load(`${name}/${name === "legacy" ? "session.export.json" : "target.checkpoint"}`);
      check(input !== undefined, "Missing frozen historical save");
      const save = name === "legacy" ? new TextDecoder().decode(input) : input;
      const checked = await m.inspectSave(save);
      const current = publications.find(p=>p.artifactId === checked.artifact);
      check(current !== undefined, "Missing exact current artifact");
      const historyPacks = publications.filter(p=>p !== current).map(p=>p.pack());
      await m.importSave(save,{execution,...checked,expected_revision:null,now:100});
      reference = await openBook({pack:current.pack(),historyPacks,execution,storage:stored});
      const before = await reference.inspect();
      let shallow;
      try { shallow = await m.exportSave({execution,...checked,ancestors:0}); }
      catch (error) {
        check(error.code === "history_truncated", "Unexpected shallow context failure");
        check((await locateSave(stored,execution))?.cursor === checked.cursor,"Shallow refusal moved a durable root");
        results.push({name,contextRefused:true});
        continue;
      }
      cold = await IndexedDbStore.open(`maintenance-history-cold-${crypto.randomUUID()}`);
      receiving = api.createMaintenance({storage:cold,storeId:decodeLoaded(await cold.load(empty)).store.id});
      for (const publication of publications) await receiving.registerPublication(publication);
      await receiving.importSave(shallow,{execution,...checked,expected_revision:null,now:100});
      continued = await openBook({pack:current.pack(),historyPacks,execution,storage:cold});
      check(same(exactPage(await continued.inspect()),exactPage(before)), "Historical shallow page differs: " + name);
      await choose(reference); await choose(continued);
      check(same(exactPage(await continued.inspect()),exactPage(await reference.inspect())),"Historical continuation differs: " + name);
      results.push({name,coldContinue:true,bytes:shallow.length});
    } finally {
      await Promise.all([reference?.close(),continued?.close(),m.close(),receiving?.close()]);
      stored.close(); cold?.close(); for (const publication of publications) publication.free();
    }
  }
  return results;
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
