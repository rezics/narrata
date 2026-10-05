import { runConformanceCase } from "../test/support/conformance";
import { indexedDbFixture } from "../test/support/store-fixture";

declare global {
  var runStorageConformance: (name: string) => Promise<void>;
}

globalThis.runStorageConformance = (name) => runConformanceCase(name,
  () => indexedDbFixture(`conformance-${crypto.randomUUID()}`, indexedDB));
document.querySelector("#status")!.textContent = "ready";
