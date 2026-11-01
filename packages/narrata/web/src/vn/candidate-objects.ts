import type { PreparedProgramSource } from "../transport.js";

export type CandidateObjectStatus = "fetched" | "checked" | "unavailable" | "budget" | "cancelled" | "corrupt";
export interface CandidateObjectBytes {
  readonly object: string;
  readonly bytes: Uint8Array;
}


/** Optional VN object cache; it is absent from the Gamebook import graph. */
export class CandidateObjects implements PreparedProgramSource {
  private closed = false;
  private readonly preparedObjects = new Map<string, { readonly bytes: Uint8Array; status: "fetched" | "checked" }>();
  private preparedBytes = 0;
  private preparedGeneration = 0;
  private preparedRequestCount = 0;
  private preparedFetchedBytes = 0;
  private reservedFetchBytes = 0;
  private readonly optionalOperations = new Set<AbortController>();
  private activeOptionalRequests = 0;
  private readonly optionalWaiters: Array<{
    readonly signal: AbortSignal;
    readonly resolve: (release: () => void) => void;
    readonly reject: (error: unknown) => void;
    abort: () => void;
  }> = [];

  constructor(private readonly loaderFor: (artifact: string) => ((id: string, signal?: AbortSignal) => Promise<Uint8Array>) | undefined, private readonly closedMessage = "The VN reader is closed.") {}

  /**
   * Fetch optional candidate objects outside the narrative queue. Bytes remain host-owned
   * until the ordinary reader operation requests that exact artifact/object pair.
   */
  async prepareObjects(
    artifact: string,
    ids: readonly string[],
    signal: AbortSignal,
  ): Promise<readonly { readonly object: string; readonly status: CandidateObjectStatus }[]> {
    this.assertOpen();
    if (!/^artifact:[0-9a-f]{64}$/.test(artifact)) throw new Error("Invalid artifact for optional program preparation");
    const unique = [...new Set(ids)];
    if (unique.some(id => !/^object:[0-9a-f]{64}$/.test(id))) throw new Error("Invalid object for optional program preparation");
    const loader = this.loaderFor(artifact);
    const generation = this.preparedGeneration;
    const outcomes = new Map<string, CandidateObjectStatus>();
    for (const id of unique) {
      const cached = this.preparedObjects.get(`${artifact}\0${id}`);
      if (cached) outcomes.set(id, cached.status);
    }
    const requested = unique.filter(id => !this.preparedObjects.has(`${artifact}\0${id}`));
    const allowance = Math.max(0, Math.min(16, 128 - this.preparedRequestCount));
    const work = requested.slice(0, allowance);
    for (const id of requested.slice(allowance)) outcomes.set(id, "budget");
    this.preparedRequestCount += work.length;
    const inserted = new Set<string>();
    const releaseInserted = () => {
      for (const key of inserted) {
        const cached = this.preparedObjects.get(key);
        if (cached) {
          this.preparedObjects.delete(key);
          this.preparedBytes -= cached.bytes.byteLength;
        }
      }
      inserted.clear();
    };
    if (!loader) {
      for (const id of work) outcomes.set(id, "unavailable");
      return unique.map(object => ({ object, status: outcomes.get(object) ?? "fetched" }));
    }
    const operation = new AbortController();
    const abortOperation = () => operation.abort();
    signal.addEventListener("abort", abortOperation, { once: true });
    if (signal.aborted || this.closed) operation.abort();
    this.optionalOperations.add(operation);
    const operationSignal = operation.signal;
    operationSignal.addEventListener("abort", releaseInserted, { once: true });
    let next = 0;
    const worker = async () => {
      while (!operationSignal.aborted && !this.closed) {
        const id = work[next++];
        if (!id) return;
        try {
          const bytes = await this.fetchOptional(loader, id, operationSignal);
          this.assertOpen();
          if (operationSignal.aborted || generation !== this.preparedGeneration) { outcomes.set(id, "cancelled"); continue; }
          const key = `${artifact}\0${id}`;
          if (this.preparedObjects.has(key)) { outcomes.set(id, this.preparedObjects.get(key)!.status); continue; }
          if (!(bytes instanceof Uint8Array)) {
            outcomes.set(id, "corrupt");
            continue;
          }
          if (bytes.byteLength < 1 || bytes.byteLength > 8 * 1024 * 1024
            || this.preparedFetchedBytes > 64 * 1024 * 1024 || this.preparedBytes + bytes.byteLength > 16 * 1024 * 1024) {
            outcomes.set(id, "budget");
            continue;
          }
          outcomes.set(id, "fetched");
          this.preparedObjects.set(key, { bytes: bytes.slice(), status: "fetched" });
          this.preparedBytes += bytes.byteLength;
          inserted.add(key);
        } catch (error) {
          outcomes.set(id, operationSignal.aborted ? "cancelled" : error instanceof CandidateByteBudgetError ? "budget" : "unavailable");
        }
      }
    };
    await Promise.all(Array.from({ length: Math.min(3, work.length) }, () => worker()));
    operationSignal.removeEventListener("abort", releaseInserted);
    signal.removeEventListener("abort", abortOperation);
    this.optionalOperations.delete(operation);
    if (operationSignal.aborted || generation !== this.preparedGeneration) releaseInserted();
    for (const id of work) if (!outcomes.has(id)) outcomes.set(id, operationSignal.aborted ? "cancelled" : "unavailable");
    return unique.map(object => ({ object, status: outcomes.get(object) ?? "fetched" }));
  }

  candidateObjectBytes(artifact: string, ids: readonly string[]): readonly CandidateObjectBytes[] {
    this.assertOpen();
    return [...new Set(ids)].flatMap(object => {
      const cached = this.preparedObjects.get(`${artifact}\0${object}`);
      return cached ? [{ object, bytes: cached.bytes.slice() }] : [];
    });
  }

  markCandidateObjectsChecked(artifact: string, ids: readonly string[]): void {
    for (const id of ids) {
      const key = `${artifact}\0${id}`;
      const cached = this.preparedObjects.get(key);
      if (cached) this.preparedObjects.set(key, { ...cached, status: "checked" });
    }
  }

  discardCandidateObjects(artifact: string, ids: readonly string[]): void {
    for (const id of ids) {
      const key = `${artifact}\0${id}`;
      const cached = this.preparedObjects.get(key);
      if (cached) {
        this.preparedObjects.delete(key);
        this.preparedBytes -= cached.bytes.byteLength;
      }
    }
  }

  /** Drop optional bytes when the player changes authorization or content scope. */
  clearPreparedObjects(): void {
    this.preparedGeneration++;
    for (const operation of this.optionalOperations) operation.abort();
    this.optionalOperations.clear();
    this.preparedObjects.clear();
    this.preparedBytes = 0;
  }

  private async fetchOptional(loader: (id: string, signal?: AbortSignal) => Promise<Uint8Array>, id: string, signal: AbortSignal): Promise<Uint8Array> {
    const release = await this.acquireOptional(signal);
    const reserved = 8 * 1024 * 1024;
    if (signal.aborted) {
      release();
      throw new DOMException("Optional program preparation was cancelled.", "AbortError");
    }
    // The host does not supply sizes in advance. Reserve the per-object maximum so
    // concurrent, duplicate and cancelled requests cannot spend the same byte credit.
    if (this.preparedFetchedBytes + this.reservedFetchBytes + reserved > 64 * 1024 * 1024) {
      release();
      throw new CandidateByteBudgetError("Optional program requests exceed their cumulative byte budget.");
    }
    this.reservedFetchBytes += reserved;
    let loading: Promise<Uint8Array> | undefined;
    let released = false;
    const settle = (bytes?: Uint8Array) => {
      if (released) return;
      released = true;
      this.reservedFetchBytes -= reserved;
      if (bytes instanceof Uint8Array) this.preparedFetchedBytes += bytes.byteLength;
      release();
    };
    try {
      loading = Promise.resolve(loader(id, signal)).then(bytes => { settle(bytes); return bytes; }, error => { settle(); throw error; });
      return await raceAbort(loading, signal);
    } catch (error) {
      // A cancelled caller stops waiting, but its host request keeps the reservation
      // and the concurrency slot until the original promise actually settles.
      if (!loading) settle();
      throw error;
    }
  }

  private acquireOptional(signal: AbortSignal): Promise<() => void> {
    if (signal.aborted) return Promise.reject(new DOMException("Optional program preparation was cancelled.", "AbortError"));
    if (this.closed) return Promise.reject(new Error(this.closedMessage));
    if (this.activeOptionalRequests < 3) {
      this.activeOptionalRequests++;
      return Promise.resolve(() => this.releaseOptional());
    }
    return new Promise((resolve, reject) => {
      const waiter = {
        signal,
        resolve,
        reject,
        abort: () => {
          const index = this.optionalWaiters.indexOf(waiter);
          if (index >= 0) this.optionalWaiters.splice(index, 1);
          reject(new DOMException("Optional program preparation was cancelled.", "AbortError"));
        },
      };
      signal.addEventListener("abort", waiter.abort, { once: true });
      this.optionalWaiters.push(waiter);
    });
  }

  private releaseOptional(): void {
    this.activeOptionalRequests--;
    while (this.optionalWaiters.length > 0 && this.activeOptionalRequests < 3) {
      const waiter = this.optionalWaiters.shift();
      if (!waiter) return;
      waiter.signal.removeEventListener("abort", waiter.abort);
      if (waiter.signal.aborted) {
        waiter.reject(new DOMException("Optional program preparation was cancelled.", "AbortError"));
        continue;
      }
      this.activeOptionalRequests++;
      waiter.resolve(() => this.releaseOptional());
    }
  }


  take(artifact: string, object: string): Uint8Array | undefined {
    const key = artifact + "\0" + object;
    const value = this.preparedObjects.get(key);
    if (!value) return undefined;
    this.preparedObjects.delete(key);
    this.preparedBytes -= value.bytes.byteLength;
    return value.bytes;
  }

  close(): void {
    this.closed = true;
    this.clearPreparedObjects();
  }

  private assertOpen(): void {
    if (this.closed) throw new Error(this.closedMessage);
  }
}

class CandidateByteBudgetError extends Error {}

function raceAbort<T>(promise: Promise<T>, signal: AbortSignal): Promise<T> {
  return new Promise<T>((resolve, reject) => {
    let settled = false;
    const cleanup = () => signal.removeEventListener("abort", abort);
    const abort = () => {
      if (settled) return;
      settled = true;
      cleanup();
      reject(new DOMException("Optional program preparation was cancelled.", "AbortError"));
    };
    if (signal.aborted) { abort(); return; }
    signal.addEventListener("abort", abort, { once: true });
    promise.then(value => {
      if (settled || signal.aborted) return;
      settled = true;
      cleanup();
      resolve(value);
    }, error => {
      if (settled) return;
      settled = true;
      cleanup();
      reject(error);
    });
  });
}
