import { describe, expect, it } from "vitest";

import { type CacheHandle, type CacheStore, StorageHost, StoreSuperseded } from "../src/host";

const bytes = (...values: number[]) => new Uint8Array(values);

/** A cache in which each operation lacks data until it is loaded once; records what the host does. */
class ScriptedCache implements CacheHandle {
  events: string[] = [];
  loaded = new Set<number>();
  pending: Uint8Array | undefined;
  request: Uint8Array | undefined;
  superseded = false;

  takeRequest() {
    const request = this.request;
    this.request = undefined;
    return request;
  }

  load(loaded: Uint8Array) {
    this.events.push(`load ${loaded[0]}`);
    this.loaded.add(loaded[0]!);
    return !this.superseded;
  }

  unconfirmed() {
    return this.pending;
  }

  confirm(reply: Uint8Array) {
    this.events.push(`confirm ${reply[0]}`);
    this.pending = undefined;
    return !this.superseded;
  }

  /** An operation that needs its data loaded, then writes one batch. */
  operation(result: number) {
    return () => {
      this.events.push(`run ${result}`);
      if (!this.loaded.has(result)) {
        this.request = bytes(result);
        throw new Error("storage has not loaded what the call reads");
      }
      this.pending = bytes(result);
      return result;
    };
  }
}

class RecordingStore implements CacheStore {
  constructor(private readonly events: string[]) {}

  async load(request: Uint8Array) {
    this.events.push(`fetch ${request[0]}`);
    await Promise.resolve();
    return request;
  }

  async persist(flush: Uint8Array) {
    this.events.push(`persist ${flush[0]}`);
    await Promise.resolve();
    return flush;
  }
}

describe("StorageHost", () => {
  it("loads what an operation lacked, runs it again, and persists before resolving", async () => {
    const cache = new ScriptedCache();
    const host = new StorageHost(new RecordingStore(cache.events), cache);
    expect(await host.run(cache.operation(7))).toBe(7);
    expect(cache.events).toEqual(["run 7", "fetch 7", "load 7", "run 7", "persist 7", "confirm 7"]);
  });

  it("passes on failures that need no load", async () => {
    const cache = new ScriptedCache();
    const host = new StorageHost(new RecordingStore(cache.events), cache);
    const failure = new Error("the step refused the input");
    await expect(
      host.run(() => {
        throw failure;
      }),
    ).rejects.toBe(failure);
    expect(cache.events).toEqual([]);
  });

  it("withholds the result when another writer came first", async () => {
    const cache = new ScriptedCache();
    cache.loaded.add(3);
    cache.superseded = true;
    const host = new StorageHost(new RecordingStore(cache.events), cache);
    await expect(host.run(cache.operation(3))).rejects.toThrow(StoreSuperseded);
    expect(cache.events).toEqual(["run 3", "persist 3", "confirm 3"]);

    await expect(host.run(cache.operation(4))).rejects.toThrow(StoreSuperseded);
  });

  it("runs operations one at a time, so no load overlaps a flush", async () => {
    const cache = new ScriptedCache();
    const host = new StorageHost(new RecordingStore(cache.events), cache);
    const first = host.run(cache.operation(1));
    const second = host.run(cache.operation(2));
    expect(await Promise.all([first, second])).toEqual([1, 2]);
    expect(cache.events).toEqual([
      "run 1",
      "fetch 1",
      "load 1",
      "run 1",
      "persist 1",
      "confirm 1",
      "run 2",
      "fetch 2",
      "load 2",
      "run 2",
      "persist 2",
      "confirm 2",
    ]);
  });

  it("gives up when an operation keeps lacking data", async () => {
    const cache = new ScriptedCache();
    const host = new StorageHost(new RecordingStore(cache.events), cache, 3);
    await expect(
      host.run(() => {
        cache.request = bytes(0);
        throw new Error("never enough");
      }),
    ).rejects.toThrow("after 3 loads");
  });
});
