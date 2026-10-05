import { mkdir, mkdtemp, rm } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { expect, it } from "vitest";
import { PostgresStore } from "../server/postgres";
import { encodeFlush } from "../src/protocol";
import { batch, key, load } from "./support/conformance";
import { ddl, generousQuota, pgliteDatabase } from "./server-fixture";

it("retains committed PGlite state across a database shutdown and filesystem reopen", async () => {
  const parent = fileURLToPath(new URL("../../../../../.temp/server-storage/", import.meta.url));
  await mkdir(parent, { recursive: true });
  const directory = await mkdtemp(`${parent}/database-`);
  let db = await pgliteDatabase(directory);
  const id = crypto.getRandomValues(new Uint8Array(16));
  try {
    await db.exec(ddl);
    await db.query("INSERT INTO narrata_libraries (id, owner_key, work_key) VALUES ($1, 'reader', 'work')", [id]);
    const store = new PostgresStore(db, id, generousQuota);
    await store.persist(encodeFlush({ store: id, batches: [batch(0, { putKeys: [[key(), new Uint8Array([7])]] })] }));
    const before = await load(store, { keys: [key()] });
    await db.close();
    db = await pgliteDatabase(directory);
    expect(await load(new PostgresStore(db, id, generousQuota), { keys: [key()] })).toEqual(before);
  } finally { await db.close(); await rm(directory, { recursive: true, force: true }); }
});
