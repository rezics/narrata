import "fake-indexeddb/auto";
import { expect, test, vi } from "vitest";
import { checkedVnView, openVn } from "@rezics/narrata/vn";
import type { VnViewV1 } from "@rezics/narrata/vn";
import { IndexedDbStore, StoreSuperseded, type CacheStore } from "@rezics/narrata/storage";
import { openBook } from "@rezics/narrata";
import { NodeBook, VnBook } from "../dist/reader-wasm/narrata_nodes_wasm.js";
import { demoPack, execution, lazyVnPack, objectId, splitPack, vnPack } from "./fixture.js";

function advance(view: VnViewV1) {
  const interaction = view.view.interaction;
  if (interaction.kind !== "advance") throw new Error("Expected a VN dialogue safe point");
  return interaction;
}

function choice(view: VnViewV1) {
  const interaction = view.view.interaction;
  if (interaction.kind !== "choose") throw new Error("Expected a VN choice safe point");
  return interaction;
}

test("VN entry keeps dialogue and choice distinct, restores scene and history, and follows an alternate branch", async () => {
  const store = await IndexedDbStore.open(`vn-reader-${crypto.randomUUID()}`);
  let reader = await openVn({ pack: vnPack, execution, storage: store });
  try {
    const initial = await reader.inspect();
    expect(initial.version).toBe(1);
    expect(initial.view.artifact_id).toBe(reader.artifactId);
    expect(initial.view.depth).toBe(0);
    expect(advance(initial).line.unit.key).toBe("dialogue.one");
    expect(advance(initial).speaker?.key).toBe("speaker.hero");
    expect(advance(initial).voice?.key).toBe("voice/one");
    expect(initial.view.scene.background?.key).toBe("bg/room");
    expect(initial.view.scene.music?.key).toBe("music/theme");

    await expect(reader.advance(initial.view.cursor, "node:00000000000000000000000000000003")).rejects.toThrow();
    await expect(reader.choose(initial.view.cursor, "choice-point:00000000000000000000000000000000", [])).rejects.toThrow();
    expect((await reader.inspect()).view.cursor).toBe(initial.view.cursor);

    const firstCommit = await reader.advance(initial.view.cursor, advance(initial).dialogue);
    await expect(reader.advance(initial.view.cursor, advance(initial).dialogue)).rejects.toThrow(/stale/i);
    const second = await reader.inspect();
    expect(second.view.cursor).toBe(firstCommit);
    expect(second.view.depth).toBe(1);
    expect(advance(second).line.unit.key).toBe("dialogue.two");
    await reader.advance(second.view.cursor, advance(second).dialogue);

    const dialogue = await reader.inspect();
    expect(dialogue.view.depth).toBe(2);
    expect(advance(dialogue).line.unit.key).toBe("dialogue.three");
    expect(advance(dialogue).voice?.key).toBe("voice/three");
    const dialogueSave = await reader.exportSave();
    expect(dialogueSave).not.toContain("The last train is already gone.");
    expect(dialogueSave).not.toContain("https://");

    await reader.close();
    reader = await openVn({ pack: vnPack, execution, storage: store });
    const restoredDialogue = await reader.inspect();
    expect(restoredDialogue.view.artifact_id).toBe(initial.view.artifact_id);
    expect(restoredDialogue.view.cursor).toBe(dialogue.view.cursor);
    expect(restoredDialogue.view.depth).toBe(2);
    expect(advance(restoredDialogue).line.unit.key).toBe("dialogue.three");

    await reader.advance(restoredDialogue.view.cursor, advance(restoredDialogue).dialogue);
    const atChoice = await reader.inspect();
    const route = choice(atChoice);
    expect(atChoice.view.depth).toBe(3);
    expect(route.options.map(option => option.key)).toEqual(["left", "right"]);
    const provenance = await reader.historyArtifacts(atChoice.view.cursor);
    expect(provenance[atChoice.view.cursor]).toBe(initial.view.artifact_id);
    const choiceSave = await reader.exportSave();

    await reader.close();
    reader = await openVn({ pack: vnPack, execution, storage: store });
    const restoredChoice = await reader.inspect();
    expect(restoredChoice.view.artifact_id).toBe(initial.view.artifact_id);
    expect(restoredChoice.view.cursor).toBe(atChoice.view.cursor);
    expect(choice(restoredChoice).choice_point).toBe(route.choice_point);
    expect(await reader.exportSave()).toBe(choiceSave);

    const left = choice(restoredChoice).options.find(option => option.key === "left");
    const right = choice(restoredChoice).options.find(option => option.key === "right");
    if (!left || !right) throw new Error("Expected both VN branch options");
    await reader.choose(restoredChoice.view.cursor, route.choice_point, [left.id]);
    const onLeft = await reader.inspect();
    expect(advance(onLeft).line.unit.key).toBe("dialogue.four");
    expect(onLeft.view.scene.background?.key).toBe("bg/hall");
    expect(onLeft.view.scene.layers.map(layer => layer.id)).toEqual(["hero", "friend"]);
    expect(onLeft.view.scene.music?.key).toBe("music/theme");
    expect(onLeft.view.music_action?.kind).toBe("keep");

    await reader.checkout(atChoice.view.cursor);
    const backAtChoice = await reader.inspect();
    expect(choice(backAtChoice).choice_point).toBe(route.choice_point);
    await reader.choose(backAtChoice.view.cursor, route.choice_point, [right.id]);
    const onRight = await reader.inspect();
    expect(advance(onRight).line.unit.key).toBe("dialogue.four");
    expect(onRight.view.artifact_id).toBe(initial.view.artifact_id);
    expect(onRight.view.cursor).not.toBe(onLeft.view.cursor);
  } finally {
    await reader.close();
    store.close();
  }
});

test("Advance retries its exact missing graph chunk without changing the confirmed dialogue", async () => {
  const store = await IndexedDbStore.open(`vn-chunks-${crypto.randomUUID()}`);
  const parts = splitPack(lazyVnPack);
  const writes: Uint8Array[] = [];
  const storage: CacheStore = {
    load: request => store.load(request),
    persist: async batch => {
      writes.push(batch.slice());
      return store.persist(batch);
    },
  };
  const requests: string[] = [];
  let failLogicalChunk = false;
  let failedChunk: string | undefined;
  const reader = await openVn({
    manifest: parts.manifest,
    execution,
    storage,
    fetchChunk: async id => {
      requests.push(id);
      const bytes = parts.objects.get(id);
      if (!bytes) throw new Error(`Missing fixture object ${id}`);
      if (failLogicalChunk) {
        failLogicalChunk = false;
        failedChunk = id;
        throw new Error("temporary chunk outage");
      }
      return bytes;
    },
  });
  let reopened: Awaited<ReturnType<typeof openVn>> | undefined;
  try {
    const before = await reader.inspect();
    expect(advance(before).line.unit.key).toBe("dialogue.one");
    const expected = before.view.cursor;
    const dialogue = advance(before).dialogue;
    const priorLoads = new Set(requests);
    const priorWrites = writes.length;
    failLogicalChunk = true;
    await expect(reader.advance(expected, dialogue)).rejects.toThrow("temporary chunk outage");
    expect(failedChunk).toBeDefined();
    expect(new Set(parts.chunks.map(objectId)).has(failedChunk!)).toBe(true);
    expect(priorLoads.has(failedChunk!)).toBe(false);
    expect(requests.filter(id => !priorLoads.has(id))).toEqual([failedChunk]);
    expect(writes).toHaveLength(priorWrites);

    const stillCurrent = await reader.inspect();
    expect(stillCurrent.view.cursor).toBe(expected);
    expect(stillCurrent.view.depth).toBe(0);
    expect(advance(stillCurrent).line.unit.key).toBe("dialogue.one");

    reopened = await openVn({ pack: lazyVnPack, execution, storage });
    const durable = await reopened.inspect();
    expect(durable.view.cursor).toBe(expected);
    expect(durable.view.depth).toBe(0);
    expect(advance(durable).line.unit.key).toBe("dialogue.one");
    await reopened.close();
    reopened = undefined;

    await reader.advance(expected, dialogue);
    const after = await reader.inspect();
    expect(after.view.cursor).not.toBe(expected);
    expect(after.view.depth).toBe(1);
    expect(advance(after).line.unit.key).toBe("dialogue.two");
    expect(writes).toHaveLength(priorWrites + 1);

    await reader.close();
    reopened = await openVn({ pack: lazyVnPack, execution, storage });
    const cold = await reopened.inspect();
    expect(cold.view.cursor).toBe(after.view.cursor);
    expect(cold.view.depth).toBe(1);
    expect(advance(cold).line.unit.key).toBe("dialogue.two");
  } finally {
    await reader.close();
    await reopened?.close();
    store.close();
  }
});

test("VN save retry resends the same bytes and durable progress contains one transition", async () => {
  const store = await IndexedDbStore.open(`vn-save-${crypto.randomUUID()}`);
  let failOnce = false;
  const writes: Uint8Array[] = [];
  const storage: CacheStore = {
    load: request => store.load(request),
    persist: async batch => {
      writes.push(batch.slice());
      if (failOnce) {
        failOnce = false;
        throw new Error("storage response unavailable");
      }
      return store.persist(batch);
    },
  };
  let reader = await openVn({ pack: vnPack, execution, storage });
  try {
    const before = await reader.inspect();
    const first = advance(before);
    failOnce = true;
    await expect(reader.advance(before.view.cursor, first.dialogue)).rejects.toThrow("storage response unavailable");
    const pendingBytes = writes.at(-1)?.slice();
    if (!pendingBytes) throw new Error("Expected a failed storage request");
    await expect(reader.inspect()).rejects.toThrow("unknown outcome");
    expect(await reader.retrySave()).toBe(true);
    expect(writes.at(-1)).toEqual(pendingBytes);
    const confirmed = await reader.inspect();
    expect(confirmed.view.depth).toBe(1);
    expect(advance(confirmed).line.unit.key).toBe("dialogue.two");
    const confirmedCursor = confirmed.view.cursor;
    await reader.close();

    reader = await openVn({ pack: vnPack, execution, storage });
    const reopened = await reader.inspect();
    expect(reopened.view.artifact_id).toBe(confirmed.view.artifact_id);
    expect(reopened.view.cursor).toBe(confirmedCursor);
    expect(reopened.view.depth).toBe(1);
    expect(advance(reopened).line.unit.key).toBe("dialogue.two");
  } finally {
    await reader.close();
    store.close();
  }
});

test("a VN lost acknowledgement retries the persisted bytes without advancing again", async () => {
  const store = await IndexedDbStore.open(`vn-lost-ack-${crypto.randomUUID()}`);
  let loseAcknowledgement = false;
  const writes: Uint8Array[] = [];
  const storage: CacheStore = {
    load: request => store.load(request),
    persist: async batch => {
      writes.push(batch.slice());
      const reply = await store.persist(batch);
      if (loseAcknowledgement) {
        loseAcknowledgement = false;
        throw new Error("persisted write acknowledgement lost");
      }
      return reply;
    },
  };
  const advanceCalls = vi.spyOn(VnBook.prototype, "advance");
  let reader = await openVn({ pack: vnPack, execution, storage });
  try {
    const before = await reader.inspect();
    const dialogue = advance(before);
    loseAcknowledgement = true;
    await expect(reader.advance(before.view.cursor, dialogue.dialogue)).rejects.toThrow("persisted write acknowledgement lost");
    expect(advanceCalls).toHaveBeenCalledTimes(1);
    const originalFlush = writes.at(-1)?.slice();
    if (!originalFlush) throw new Error("Expected the persisted request bytes");
    const requestsBeforeRetry = writes.length;
    await expect(reader.retrySave()).rejects.toBeInstanceOf(StoreSuperseded);
    expect(writes).toHaveLength(requestsBeforeRetry + 1);
    expect(writes.at(-1)).toEqual(originalFlush);
    expect(advanceCalls).toHaveBeenCalledTimes(1);

    await reader.reload();
    const durable = await reader.inspect();
    expect(durable.view.depth).toBe(1);
    expect(advance(durable).line.unit.key).toBe("dialogue.two");
    expect(advanceCalls).toHaveBeenCalledTimes(1);
    const confirmedCursor = durable.view.cursor;
    const artifact = durable.view.artifact_id;
    await reader.close();

    reader = await openVn({ pack: vnPack, execution, storage });
    const reopened = await reader.inspect();
    expect(reopened.view.artifact_id).toBe(artifact);
    expect(reopened.view.cursor).toBe(confirmedCursor);
    expect(reopened.view.depth).toBe(1);
    expect(advance(reopened).line.unit.key).toBe("dialogue.two");
    expect(advanceCalls).toHaveBeenCalledTimes(1);
  } finally {
    await reader.close();
    advanceCalls.mockRestore();
    store.close();
  }
});

test("close waits for a deferred logical load, rejects new calls and prevents late advancement", async () => {
  const store = await IndexedDbStore.open(`vn-close-load-${crypto.randomUUID()}`);
  const parts = splitPack(lazyVnPack);
  const writes: Uint8Array[] = [];
  const storage: CacheStore = {
    load: request => store.load(request),
    persist: async batch => {
      writes.push(batch.slice());
      return store.persist(batch);
    },
  };
  let opening = true;
  let startedLoading: (() => void) | undefined;
  const loadStarted = new Promise<void>(resolve => { startedLoading = resolve; });
  let resolveLateLoad: ((bytes: Uint8Array) => void) | undefined;
  const lateLoad = new Promise<Uint8Array>(resolve => { resolveLateLoad = resolve; });
  let lateChunkId: string | undefined;
  const initialChunkIds: string[] = [];
  const fetchChunk = (id: string) => {
    if (opening) {
      initialChunkIds.push(id);
      const bytes = parts.objects.get(id);
      if (!bytes) throw new Error(`Missing fixture object ${id}`);
      return Promise.resolve(bytes);
    }
    lateChunkId = id;
    startedLoading?.();
    return lateLoad;
  };
  const reader = await openVn({ manifest: parts.manifest, execution, storage, fetchChunk });
  let reopened: Awaited<ReturnType<typeof openVn>> | undefined;
  let closePromise: Promise<void> | undefined;
  const free = vi.spyOn(VnBook.prototype, "free");
  try {
    opening = false;
    const before = await reader.inspect();
    const dialogue = advance(before);
    const writesBefore = writes.length;
    const advancePromise = reader.advance(before.view.cursor, dialogue.dialogue);
    await loadStarted;
    expect(lateChunkId).toBeDefined();
    expect(new Set(parts.chunks.map(objectId)).has(lateChunkId!)).toBe(true);
    expect(initialChunkIds).not.toContain(lateChunkId);

    let closed = false;
    closePromise = reader.close().then(() => { closed = true; });
    await expect(reader.inspect()).rejects.toThrow("closed");
    await expect(reader.advance(before.view.cursor, dialogue.dialogue)).rejects.toThrow("closed");
    await new Promise(resolve => setTimeout(resolve, 20));
    expect(closed).toBe(false);
    expect(free).not.toHaveBeenCalled();

    const lateBytes = parts.objects.get(lateChunkId!);
    if (!lateBytes) throw new Error(`Missing deferred fixture object ${lateChunkId}`);
    resolveLateLoad?.(lateBytes);
    await expect(advancePromise).rejects.toThrow("closed");
    await closePromise;
    expect(closed).toBe(true);
    expect(free).toHaveBeenCalledTimes(1);
    expect(writes).toHaveLength(writesBefore);

    reopened = await openVn({ pack: lazyVnPack, execution, storage });
    const durable = await reopened.inspect();
    expect(durable.view.cursor).toBe(before.view.cursor);
    expect(durable.view.depth).toBe(0);
    expect(advance(durable).line.unit.key).toBe("dialogue.one");
  } finally {
    if (!closePromise) closePromise = reader.close();
    const lateBytes = lateChunkId ? parts.objects.get(lateChunkId) : undefined;
    if (lateBytes) resolveLateLoad?.(lateBytes);
    if (closePromise) await closePromise;
    await reopened?.close();
    free.mockRestore();
    store.close();
  }
});

test("generated VN validator requires version 1, scene and a known interaction", async () => {
  const reader = await openVn({ pack: vnPack, execution });
  try {
    const view = await reader.inspect();
    expect(checkedVnView(view)).toBe(view);
    const wrongVersion = structuredClone(view) as { version: number };
    wrongVersion.version = 2;
    expect(() => checkedVnView(wrongVersion)).toThrow("invalid VN View v1");
    const noScene = structuredClone(view) as { view: { scene?: unknown } };
    delete noScene.view.scene;
    expect(() => checkedVnView(noScene)).toThrow("invalid VN View v1");
    const unknownInteraction = structuredClone(view) as { view: { interaction: { kind: string } } };
    unknownInteraction.view.interaction.kind = "skip";
    expect(() => checkedVnView(unknownInteraction)).toThrow("invalid VN View v1");
  } finally {
    await reader.close();
  }
});

test("the Gamebook and VN facades reject the other artifact format", async () => {
  const vnParts = splitPack(vnPack);
  const bookParts = splitPack(demoPack);
  let loads = 0;
  let writes = 0;
  let chunkLoads = 0;
  const storage: CacheStore = {
    load: async () => { loads++; return new Uint8Array(); },
    persist: async () => { writes++; return new Uint8Array(); },
  };
  await expect(openVn({ pack: demoPack, execution, storage })).rejects.toThrow();
  await expect(openBook({ pack: vnPack, execution, storage })).rejects.toThrow();
  await expect(openVn({ manifest: bookParts.manifest, execution, storage, fetchChunk: async () => { chunkLoads++; return new Uint8Array(); } })).rejects.toThrow();
  await expect(openBook({ manifest: vnParts.manifest, execution, storage, fetchChunk: async () => { chunkLoads++; return new Uint8Array(); } })).rejects.toThrow();
  expect({ loads, writes, chunkLoads }).toEqual({ loads: 0, writes: 0, chunkLoads: 0 });

  const gamebook = new NodeBook(demoPack, execution);
  const novel = new VnBook(vnPack, execution);
  try {
    await expect(Promise.resolve().then(() => novel.registerHistoryPack(demoPack))).rejects.toThrow();
    await expect(Promise.resolve().then(() => gamebook.registerHistoryPack(vnPack))).rejects.toThrow();
    await expect(Promise.resolve().then(() => novel.registerHistoryManifest(gamebook.artifact_id, bookParts.manifest))).rejects.toThrow();
    await expect(Promise.resolve().then(() => gamebook.registerHistoryManifest(novel.artifact_id, vnParts.manifest))).rejects.toThrow();
    expect(() => novel.registerHistoryPack(vnPack)).not.toThrow();
    expect(() => gamebook.registerHistoryPack(demoPack)).not.toThrow();
    expect(() => novel.registerHistoryManifest(novel.artifact_id, vnParts.manifest)).not.toThrow();
    expect(() => gamebook.registerHistoryManifest(gamebook.artifact_id, bookParts.manifest)).not.toThrow();
    expect(novel.takeRequest()).toBeUndefined();
    expect(gamebook.takeRequest()).toBeUndefined();
    expect(novel.unconfirmed()).toBeUndefined();
    expect(gamebook.unconfirmed()).toBeUndefined();
    expect(() => novel.cursor()).toThrow();
    expect(() => gamebook.cursor()).toThrow();
  } finally {
    novel.free();
    gamebook.free();
  }
});
