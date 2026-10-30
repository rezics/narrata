import { decodeReaderMap } from "@rezics/narrata/reader-map";
const stable = value => JSON.stringify(value, (_, item) => item && typeof item === "object" && !Array.isArray(item) ? Object.fromEntries(Object.entries(item).sort(([a], [b]) => a.localeCompare(b))) : item);
/** The same frozen Rust outputs are consumed from an independently installed Node/Bun
 * package and a browser Worker. No repository runtime implementation is imported. */
export async function verifyMaps(api, load) {
  const decoder = new TextDecoder();
  const publication = api.publicationFromPack(await load("story.narpack"));
  const engine = api.createEvolution();
  try {
    const query = { kind: "node", node: "node:" + "00".repeat(16) };
    const rejects = operation => { let rejected = false; try { operation(); } catch { rejected = true; } if (!rejected) throw new Error("Unsafe numeric boundary accepted"); };
    for (const bad of [2 ** 32 + 1, .5, NaN, Infinity, -Infinity, -1, 4_000_001]) {
      rejects(() => publication.relationRows("node", bad, 1));
      rejects(() => publication.relationRows("node", 0, bad));
      rejects(() => publication.checkRelationRows("node", bad, 1, []));
      rejects(() => publication.checkRelationRows("node", 0, bad, []));
      rejects(() => publication.queryRelations(query, bad, 1));
      rejects(() => publication.queryRelations(query, 0, bad));
      rejects(() => publication.checkRelationQuery(query, bad, 1, []));
      rejects(() => publication.checkRelationQuery(query, 0, bad, []));
      rejects(() => publication.relationPart(bad));
    }
    await engine.registerPublication(publication);
    for (const name of ["initial", "local-disabled", "multi", "called", "ending"]) {
      await engine.restoreSave(decoder.decode(await load(name + ".checkpoint")));
      const visits = await engine.visits();
      try {
        const expected = JSON.parse(decoder.decode(await load(name + ".map.json")));
        const actual = visits.readerMap(publication);
        decodeReaderMap(actual, { artifactId: publication.artifactId, cursor: visits.report().sources[0].cursor });
        if (stable(actual) !== stable(expected)) throw new Error("Installed reader map differs: " + name);
        if (!visits.report().sources[0].complete) throw new Error("Incomplete frozen visits");
        const allowed = visits.readerRelations(publication);
        try {
          for (let offset = 0; offset < allowed.count; offset += 16) allowed.checkRows(offset, 16, allowed.rows(offset, 16));
          for (const bad of [2 ** 32 + 1, .5, NaN, Infinity, -Infinity, -1, 4_000_001]) {
            rejects(() => allowed.rows(bad, 1)); rejects(() => allowed.rows(0, bad));
            rejects(() => allowed.checkRows(bad, 1, [])); rejects(() => allowed.checkRows(0, bad, []));
          }
        }
        finally { allowed.free(); }
      } finally { visits.free(); }
    }
    await engine.restoreSave(decoder.decode(await load("called.checkpoint")));
    const called = await engine.visits();
    await engine.restoreSave(decoder.decode(await load("multi.checkpoint")));
    const back = await engine.visits();
    const merged = back.merge(called);
    try { if (merged.report().sources.length !== 2 || merged.readerMap(publication).units.length !== 2) throw new Error("Merged cursor provenance"); }
    finally { merged.free(); }
    const limited = await engine.visits({ transitions: 0, automatic_steps: 0, object_bytes: 0 });
    try { const map = limited.readerMap(publication); if (map.sources[0].complete || !map.sources[0].missing.length || map.nodes.length) throw new Error("Pre-replay budget provenance"); }
    finally { limited.free(); }
    await engine.restoreSave(await engine.exportShallow(0));
    const shallow = await engine.visits();
    try {
      const map = shallow.readerMap(publication), sparse = shallow.readerRelations(publication);
      try { if (map.sources[0].complete || !map.sources[0].missing.length || sparse.count !== 0) throw new Error("Shallow map provenance"); sparse.checkRows(0, 1, []); }
      finally { sparse.free(); }
    } finally { shallow.free(); }
    let rejected = false;
    try { await engine.restoreSave("00".repeat(100)); } catch { rejected = true; }
    if (!rejected) throw new Error("Corrupt save accepted");
    const proposed = api.publicationFromPack(await load("extra/proposals/story.narpack"));
    const proposalEngine = api.createEvolution();
    try {
      await proposalEngine.registerPublication(proposed);
      await proposalEngine.restoreSave(decoder.decode(await load("extra/proposals/session.export.json")));
      const visits = await proposalEngine.visits();
      try {
        const events = visits.report().sources.flatMap(source => source.segments.flatMap(segment => segment.events));
        const dynamic = new Set(events.filter(event => event.action.kind === "visit" && !event.action.static_node).map(event => event.node));
        if (!dynamic.size || visits.readerMap(proposed).nodes.some(node => dynamic.has(node.node))) throw new Error("Dynamic presentation became static topology");
        const gate = "node:00000000000000000000000000000001";
        if (events.filter(event => event.action.kind === "visit" && event.node === gate).length < 2 || visits.readerMap(proposed).nodes.filter(node => node.node === gate).length !== 1) throw new Error("Loop visits must retain one revealed position");
      } finally { visits.free(); }
    } finally { await proposalEngine.close(); proposed.free(); }
    const old = api.publicationFromPack(await load("extra/evolution/source.narpack"));
    let candidate = api.publicationFromPack(await load("extra/evolution/target.narpack"));
    const plan = api.planEvolution(old, candidate, decoder.decode(await load("extra/evolution/declarations.json")));
    const migration = api.createEvolution();
    try {
      candidate = candidate.withEvolution(old, plan);
      await migration.registerEvolution(old, candidate);
      const checkpoint = [...await load("extra/evolution/target.checkpoint")].map(byte => byte.toString(16).padStart(2, "0")).join("");
      await migration.restoreSave(checkpoint);
      const visits = await migration.visits();
      try {
        const source = visits.report().sources[0], map = visits.readerMap(candidate);
        if (!source.complete || source.segments.length !== 2 || source.segments[0].artifact_id !== old.artifactId || source.segments[1].artifact_id !== candidate.artifactId) throw new Error("Migration source ownership");
        if (map.nodes.length !== 1 || map.nodes[0].node !== "node:00000000000000000000000000000014") throw new Error("Old visits moved into target projection");
        const sparse = visits.readerRelations(candidate); try { sparse.checkRows(0, 256, sparse.rows(0, 256)); } finally { sparse.free(); }
      } finally { visits.free(); }
      const missing = api.createEvolution();
      try {
        await missing.registerPublication(old); await missing.registerPublication(candidate);
        await missing.restoreSave(checkpoint);
        const visits = await missing.visits();
        try { const map = visits.readerMap(candidate); if (map.sources[0].complete || map.nodes.length || !map.sources[0].missing.some(item => item.kind === "descriptor")) throw new Error("Missing migration evidence became complete target visits"); }
        finally { visits.free(); }
      } finally { await missing.close(); }
    } finally { await migration.close(); plan.free(); old.free(); candidate.free(); }
    return true;
  } finally { await engine.close(); publication.free(); }
}
