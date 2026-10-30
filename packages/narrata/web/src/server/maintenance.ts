import { LibraryQuotaExceeded, LibraryUnavailable, type PostgresStore } from "./postgres.js";
import { WriteRejected } from "../storage.js";
import { loadAuthoring } from "./index.js";
import type { Maintenance, MaintenanceLimits } from "../maintenance/index.js";

/** The host authorizes this immutable PostgresStore before calling. No SQL migration or
 * account lookup occurs here. Usage and quota come from the same reference store. */
export async function openLibraryMaintenance(store: PostgresStore, limits?: MaintenanceLimits): Promise<Maintenance> {
  const observed = await store.usage();
  const authoring = await loadAuthoring();
  return authoring.createMaintenance({ storeId: observed.store.id, ...(limits ? { limits } : {}), storage: {
    load: request => store.load(request),
    persist: async bytes => {
      try { return await store.persist(bytes); }
      catch (error) {
        if (error instanceof LibraryQuotaExceeded) throw new WriteRejected("quota", error.message);
        if (error instanceof LibraryUnavailable) throw new WriteRejected("unavailable", error.message);
        throw error;
      }
    },
  } });
}
