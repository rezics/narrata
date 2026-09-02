export interface StoredObject {
  readonly id: Uint8Array;
  readonly bytes: Uint8Array;
}

export interface RefMutation {
  readonly key: string;
  readonly expectedRevision: number | null;
  readonly nextObjectId: Uint8Array | null;
}

export interface StoredRef {
  readonly key: string;
  readonly revision: number;
  readonly objectId: Uint8Array;
}

export interface BrowserStoreLimits {
  readonly maxObjectBytes: number;
  readonly maxObjectsPerCommit: number;
  readonly maxCommitBytes: number;
}

const DEFAULT_LIMITS: BrowserStoreLimits = {
  maxObjectBytes: 128 * 1024 * 1024,
  maxObjectsPerCommit: 100_000,
  maxCommitBytes: 512 * 1024 * 1024,
};

/** Stores immutable objects and CAS refs in one IndexedDB transaction. */
export class NarrataIndexedDbStore {
  readonly #database: IDBDatabase;
  readonly #limits: BrowserStoreLimits;

  private constructor(database: IDBDatabase, limits: BrowserStoreLimits) {
    this.#database = database;
    this.#limits = limits;
  }

  static async open(
    name = "narrata",
    limits: BrowserStoreLimits = DEFAULT_LIMITS,
  ): Promise<NarrataIndexedDbStore> {
    const request = indexedDB.open(name, 1);
    request.onupgradeneeded = () => {
      request.result.createObjectStore("objects");
      request.result.createObjectStore("refs", { keyPath: "key" });
    };
    return new NarrataIndexedDbStore(await idbRequest(request), limits);
  }

  async getObject(id: Uint8Array): Promise<Uint8Array | null> {
    const transaction = this.#database.transaction("objects", "readonly");
    const value = await idbRequest<ArrayBuffer | undefined>(
      transaction.objectStore("objects").get(hex(id)),
    );
    await transactionDone(transaction);
    return value === undefined ? null : new Uint8Array(value);
  }

  async readRef(key: string): Promise<StoredRef | null> {
    const transaction = this.#database.transaction("refs", "readonly");
    const value = await idbRequest<StoredRef | undefined>(
      transaction.objectStore("refs").get(key),
    );
    await transactionDone(transaction);
    return value ?? null;
  }

  async commit(objects: readonly StoredObject[], refs: readonly RefMutation[]): Promise<void> {
    this.#validateLimits(objects);
    const transaction = this.#database.transaction(["objects", "refs"], "readwrite");
    const objectStore = transaction.objectStore("objects");
    const refStore = transaction.objectStore("refs");
    try {
      for (const mutation of refs) {
        const existing = await idbRequest<StoredRef | undefined>(refStore.get(mutation.key));
        const actual = existing?.revision ?? null;
        if (actual !== mutation.expectedRevision) {
          throw new Error(`Ref '${mutation.key}' changed before this save could be published.`);
        }
      }
      for (const object of objects) {
        const key = hex(object.id);
        const existing = await idbRequest<ArrayBuffer | undefined>(objectStore.get(key));
        if (existing !== undefined && !sameBytes(new Uint8Array(existing), object.bytes)) {
          throw new Error(`Object '${key}' has conflicting bytes.`);
        }
        if (existing === undefined) objectStore.add(object.bytes.slice().buffer, key);
      }
      for (const mutation of refs) {
        if (mutation.nextObjectId === null) {
          refStore.delete(mutation.key);
        } else {
          refStore.put({
            key: mutation.key,
            revision: (mutation.expectedRevision ?? 0) + 1,
            objectId: mutation.nextObjectId.slice(),
          } satisfies StoredRef);
        }
      }
      await transactionDone(transaction);
    } catch (error) {
      transaction.abort();
      throw error;
    }
  }

  close(): void {
    this.#database.close();
  }

  #validateLimits(objects: readonly StoredObject[]): void {
    if (objects.length > this.#limits.maxObjectsPerCommit) {
      throw new RangeError("This save contains more objects than the browser store allows.");
    }
    let total = 0;
    for (const object of objects) {
      if (object.bytes.byteLength > this.#limits.maxObjectBytes) {
        throw new RangeError("A save object exceeds the browser store size limit.");
      }
      total += object.bytes.byteLength;
    }
    if (total > this.#limits.maxCommitBytes) {
      throw new RangeError("This save exceeds the browser store transaction size limit.");
    }
  }
}

function idbRequest<T>(request: IDBRequest<T>): Promise<T> {
  return new Promise((resolve, reject) => {
    request.onsuccess = () => resolve(request.result);
    request.onerror = () => reject(request.error ?? new Error("IndexedDB request failed."));
  });
}

function transactionDone(transaction: IDBTransaction): Promise<void> {
  return new Promise((resolve, reject) => {
    transaction.oncomplete = () => resolve();
    transaction.onabort = () => reject(transaction.error ?? new Error("IndexedDB transaction was aborted."));
    transaction.onerror = () => reject(transaction.error ?? new Error("IndexedDB transaction failed."));
  });
}

function hex(bytes: Uint8Array): string {
  return Array.from(bytes, (value) => value.toString(16).padStart(2, "0")).join("");
}

function sameBytes(left: Uint8Array, right: Uint8Array): boolean {
  return left.length === right.length && left.every((value, index) => value === right[index]);
}
