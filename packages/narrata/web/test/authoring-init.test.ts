import { Worker } from "node:worker_threads";
import { expect, test } from "vitest";

// Each Worker owns fresh JS/Wasm module state, independent of other authoring tests.
test.each(["retry", "concurrent"])("authoring initialization in an independent realm: %s", async mode => {
  const result = await new Promise<string>((resolve, reject) => {
    const worker = new Worker(`
      const { parentPort, workerData } = require("node:worker_threads");
      (async () => {
        const assert = (await import("node:assert")).strict;
        const { loadAuthoring } = await import(workerData.entry);
        if (workerData.mode === "retry") {
          await assert.rejects(loadAuthoring(new Uint8Array([0, 1, 2, 3])));
          const api = await loadAuthoring();
          assert.equal(api.validateRecord(new TextEncoder().encode("{}")).record, undefined);
        } else {
          const first = loadAuthoring();
          const second = loadAuthoring(new Uint8Array([0, 1, 2, 3]));
          const [a, b] = await Promise.all([first, second]);
          assert.equal(a, b);
          assert.equal(a.validateRecord(new TextEncoder().encode("{}")).record, undefined);
        }
        parentPort.postMessage("ok");
      })().catch(error => { throw error; });
    `, { eval: true, workerData: { mode, entry: new URL("../dist/server/index.js", import.meta.url).href } });
    worker.once("message", resolve);
    worker.once("error", reject);
    worker.once("exit", code => { if (code !== 0) reject(new Error(`Authoring realm exited ${code}`)); });
  });
  expect(result).toBe("ok");
});
