/// <reference types="vite/client" />

import type { VnPlayerController } from "@rezics/narrata/vn/player";
import type { VnReader, VnViewV1 } from "@rezics/narrata/vn";
import type { DemoContext, DemoControls, DemoMetrics } from "./player-host.js";

declare global {
  interface Window {
    __vnTestHooks?: {
      readonly metrics: DemoMetrics;
      readonly controls: DemoControls;
      readonly reader: VnReader;
      readonly controller: VnPlayerController<DemoContext>;
      switchContext(scope: string): Promise<void>;
      save(): Promise<string>;
      currentView(): Promise<VnViewV1>;
      close(): Promise<void>;
    };
  }
}
