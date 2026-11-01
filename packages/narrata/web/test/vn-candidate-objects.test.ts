import { expect, test } from "vitest";
import { CandidateObjects } from "../dist/vn/candidate-objects.js";

const artifact = `artifact:${"a".repeat(64)}`;
const object = `object:${"b".repeat(64)}`;

test("cumulative byte credit stops requests before fetching beyond the lifetime allowance", async () => {
  const bytes = new Uint8Array(8 * 1024 * 1024);
  let calls = 0;
  const cache = new CandidateObjects(() => async () => { calls++; return bytes; });
  try {
    for (let round = 0; round < 8; round++) {
      expect((await cache.prepareObjects(artifact, [object], new AbortController().signal))[0]?.status).toBe("fetched");
      cache.clearPreparedObjects();
    }
    expect((await cache.prepareObjects(artifact, [object], new AbortController().signal))[0]?.status).toBe("budget");
    expect(calls).toBe(8);
  } finally { cache.close(); }
});

test("duplicate concurrent responses debit bytes even when one already filled the cache", async () => {
  const bytes = new Uint8Array(4 * 1024 * 1024);
  let calls = 0;
  const cache = new CandidateObjects(() => async () => { calls++; await Promise.resolve(); return bytes; });
  try {
    let result;
    for (let round = 0; round < 12; round++) {
      result = await Promise.all([
        cache.prepareObjects(artifact, [object], new AbortController().signal),
        cache.prepareObjects(artifact, [object], new AbortController().signal),
      ]);
      cache.clearPreparedObjects();
    }
    expect(calls * bytes.byteLength).toBeLessThanOrEqual(64 * 1024 * 1024);
    expect(result?.flat().every(item => item.status === "budget")).toBe(true);
  } finally { cache.close(); }
});

test("late responses still spend byte credit after their caller has cancelled", async () => {
  const bytes = new Uint8Array(8 * 1024 * 1024);
  let calls = 0;
  let started: (() => void) | undefined;
  const replies: Array<(bytes: Uint8Array) => void> = [];
  const cache = new CandidateObjects(() => async () => {
    calls++;
    started?.();
    return new Promise<Uint8Array>(resolve => { replies.push(resolve); });
  });
  try {
    let result;
    for (let round = 0; round < 9; round++) {
      const controller = new AbortController();
      const loading = new Promise<void>(resolve => { started = resolve; });
      const preparation = cache.prepareObjects(artifact, [object], controller.signal);
      await Promise.race([loading, preparation]);
      controller.abort();
      result = await preparation;
      replies.shift()?.(bytes);
      await new Promise(resolve => setTimeout(resolve, 0));
      cache.clearPreparedObjects();
    }
    expect(calls).toBe(8);
    expect(result?.[0]?.status).toBe("budget");
  } finally { cache.close(); }
});
