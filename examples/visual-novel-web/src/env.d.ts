/// <reference types="vite/client" />

import type { VnPlayerController } from "@rezics/narrata/vn/player";
import type { VnReader, VnViewV1 } from "@rezics/narrata/vn";
import type {
  DemoCandidateInspection,
  DemoCandidateTransfer,
  DemoContentDraft,
  DemoContext,
  DemoControls,
  DemoMediaOverride,
  DemoMetrics,
} from "./player-host.js";

declare global {
  interface Window {
    __vnTestHooks?: {
      readonly metrics: DemoMetrics;
      readonly controls: DemoControls;
      readonly reader: VnReader;
      readonly controller: VnPlayerController<DemoContext>;
      readonly isClosed: boolean;
      switchContext(scope: string): Promise<void>;
      save(): Promise<string>;
      currentView(): Promise<VnViewV1>;
      clearPreparedCaches(): Promise<void>;
      openCandidate(
        candidate: DemoCandidateTransfer,
        contentDrafts: readonly (readonly [string, DemoContentDraft])[],
        mediaOverrides: readonly DemoMediaOverride[],
        maxCandidateRequests?: number,
      ): Promise<number>;
      candidateState(id: number): DemoCandidateInspection;
      chooseCandidate(id: number, optionIndex?: number): Promise<void>;
      switchCandidateContext(id: number, scope: string): Promise<void>;
      closeCandidate(id: number): Promise<void>;
      close(): Promise<void>;
    };
    __vnAuthorHooks?: {
      candidate(): string | undefined;
      snapshot(): string | undefined;
      jobs(): readonly {
        readonly artifactId: string;
        readonly snapshotId: string;
        readonly status: string;
        readonly closeStarted: boolean;
        readonly readerClosed: boolean;
        readonly candidateFreed: boolean;
        readonly draftsFreed: boolean;
        readonly aborts: number;
        readonly releases: number;
        readonly installed: boolean;
      }[];
      closePreview(): Promise<void>;
    };
  }
}
