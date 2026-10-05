import type { HostReads } from "../src/index";

/** A page operation's result, or the name and message of the error it failed with. */
export type Outcome<T> = { ok: T } | { error: string; message: string };

/** What the test page exposes as `globalThis.counter`. */
export interface CounterPage {
  /** Reloads the session from the store, creating it at 0 in an empty store. */
  open(): Promise<Outcome<number>>;
  advance(increment: number): Promise<Outcome<number>>;
  save(): Promise<Outcome<void>>;
  reads(): HostReads;
  export(): Promise<number[]>;
  /** Deletes the database, as an eviction does, and imports `exported` into a new one. */
  restore(exported: number[]): Promise<void>;
  requestPersistence(): Promise<boolean>;
}
