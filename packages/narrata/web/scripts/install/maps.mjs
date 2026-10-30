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
    return true;
  } finally { await engine.close(); publication.free(); }
}
