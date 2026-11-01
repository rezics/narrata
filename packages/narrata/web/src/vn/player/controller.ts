import { contentItemKey } from "../../content/index.js";
import type { Content, ResolveContext } from "../../content/index.js";
import type { CommitId, MediaKind, VnInteractionV1, VnViewV1, ViewScalar } from "../../generated/vn-view.js";
import type { VnReader } from "../runtime.js";
import { resolveVnText, VnTextUnavailableError, type VnTextResolver } from "./content.js";
import {
  checkedVnMediaResolution,
  decodedMediaBytes,
  defaultVnMediaLimits,
  expectedKind,
  mediaSlotKey,
  releaseVnMediaResolution,
  sameMedia,
  variantId,
  VnMediaBoundaryError,
  VnMediaUnavailableError,
  type VnMediaLimits,
  type VnMediaPresenter,
  type VnMediaRequest,
  type VnMediaResolution,
  type VnMediaResolver,
  type VnMediaRole,
  type VnMediaSlot,
  type VnPreparedAsset,
  type VnPresentedMedia,
} from "./media.js";

export type VnEntryReason = "open" | "forward" | "checkout" | "restore" | "context" | "audio-retry" | "refresh";
export type VnPlayerStatus = "loading" | "ready" | "submitting" | "preparing" | "unavailable" | "logical-error" | "save-unknown" | "reload-required" | "closed";
export type VnSaveStatus = "saved" | "saving" | "unknown";
export type VnSoundStatus = "off" | "on" | "unavailable";

export interface VnPlayerContext<HostContext> {
  readonly media: HostContext;
  readonly content: ResolveContext;
}

export type VnPlayerFailure =
  | { readonly kind: "resource"; readonly issue: "unavailable" | "budget" | "invalid" | "text"; readonly retryable: true }
  | { readonly kind: "logical"; readonly action: "advance" | "choose" | "checkout"; readonly retryable: true }
  | { readonly kind: "save-unknown"; readonly retryable: true }
  | { readonly kind: "reload-required"; readonly retryable: false };

export interface VnPresentedView {
  readonly view: VnViewV1;
  readonly texts: ReadonlyMap<string, string>;
  readonly assets: readonly VnPreparedAsset[];
  readonly audioIssues: readonly VnMediaRole[];
  readonly entry: VnEntryReason;
}

export interface VnPlayerSnapshot {
  readonly status: VnPlayerStatus;
  readonly saveStatus: VnSaveStatus;
  readonly soundStatus: VnSoundStatus;
  readonly audioLoading: boolean;
  readonly screen?: VnPresentedView;
  /** Current logical target, which may be ahead of `screen` while resources are preparing. */
  readonly target?: VnViewV1;
  readonly failure?: VnPlayerFailure;
}

export interface VnPlayerOptions<HostContext> {
  readonly reader: VnReader;
  readonly text: VnTextResolver;
  readonly resolveMedia: VnMediaResolver<HostContext>;
  readonly presentMedia: VnMediaPresenter;
  readonly context: VnPlayerContext<HostContext>;
  /** The host chooses a presentation variant; the VN format never stores it. */
  readonly selectVariant: (slot: VnMediaSlot, view: VnViewV1) => string;
  readonly onLateDiscard?: (request: VnMediaRequest<HostContext>) => void;
  readonly limits?: Partial<VnMediaLimits>;
}

interface PendingInput {
  readonly expected: CommitId;
  readonly action: "advance" | "choose";
  readonly interaction: VnInteractionV1;
  readonly options?: readonly string[];
}

interface WorkResult {
  readonly assets: VnPreparedAsset[];
  readonly audioIssues: VnMediaRole[];
}

/**
 * VN-only presentation coordinator. It confirms each reader input before resolving the
 * resulting view, and owns every live host lease only until that screen is replaced.
 */
export class VnPlayerController<HostContext> {
  private readonly limits: VnMediaLimits;
  private readonly listeners = new Set<() => void>();
  private snapshot: VnPlayerSnapshot = { status: "loading", saveStatus: "saved", soundStatus: "off", audioLoading: false };
  private context: VnPlayerContext<HostContext>;
  private generation = 0;
  private requestSequence = 0;
  private operation: AbortController | undefined;
  private logicalWork: Promise<void> | undefined;
  private pendingAssets = new Set<VnPreparedAsset>();
  private audioLifecycle = new AbortController();
  private presentationGeneration = 0;
  private audioLoading = false;
  private screen: VnPresentedView | undefined;
  private target: VnViewV1 | undefined;
  private pendingInput: PendingInput | undefined;
  private pendingCheckout: CommitId | undefined;
  private pendingEntry: VnEntryReason = "forward";
  private soundStatus: VnSoundStatus = "off";
  private saveOutcomeUnknown = false;
  private started = false;
  private closed = false;

  constructor(private readonly options: VnPlayerOptions<HostContext>) {
    this.context = options.context;
    this.limits = Object.freeze({ ...defaultVnMediaLimits, ...options.limits });
    if (!positiveLimit(this.limits.maxRequests) || !positiveLimit(this.limits.maxConcurrent)
      || !positiveLimit(this.limits.maxEncodedBytesPerTarget) || !positiveLimit(this.limits.maxEncodedBytesRetained)
      || !positiveLimit(this.limits.maxDecodedBytesRetained) || !positiveLimit(this.limits.maxImageDimension)
      || !positiveLimit(this.limits.maxAudioDurationMs)) {
      throw new RangeError("VN player resource limits must be positive safe integers.");
    }
  }

  getSnapshot = (): VnPlayerSnapshot => this.snapshot;

  subscribe = (listener: () => void): (() => void) => {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  };

  async start(): Promise<void> {
    this.assertOpen();
    if (this.started) return;
    this.started = true;
    const generation = this.generation;
    try {
      const view = await this.options.reader.inspect();
      if (!this.isCurrent(generation)) return;
      await this.prepare(view, "open");
    } catch (error) {
      if (!this.isCurrent(generation)) return;
      if (isUnknownSaveOutcome(error)) {
        this.saveOutcomeUnknown = true;
        this.publish("save-unknown", undefined, undefined, { kind: "save-unknown", retryable: true }, "unknown");
      }
      else this.publish("unavailable", undefined, undefined, { kind: "resource", issue: "unavailable", retryable: true });
    }
  }

  async advance(): Promise<void> {
    const screen = this.requireInteractiveScreen();
    const interaction = screen.view.view.interaction;
    if (interaction.kind !== "advance") throw new Error("The current VN view is not an Advance safe point.");
    await this.submit({ expected: screen.view.view.cursor, action: "advance", interaction });
  }

  async choose(option: string): Promise<void> {
    const screen = this.requireInteractiveScreen();
    const interaction = screen.view.view.interaction;
    if (interaction.kind !== "choose") throw new Error("The current VN view is not a Choice safe point.");
    const selected = interaction.options.find(candidate => candidate.id === option);
    if (!selected?.enabled) throw new Error("The selected VN option is not enabled in the current view.");
    await this.submit({ expected: screen.view.view.cursor, action: "choose", interaction, options: [selected.id] });
  }

  async retryInput(): Promise<void> {
    this.assertOpen();
    if (this.snapshot.status !== "logical-error") throw new Error("There is no failed VN input to retry.");
    if (this.pendingCheckout) {
      const commit = this.pendingCheckout;
      await this.retryCheckout(commit);
      return;
    }
    if (!this.pendingInput) throw new Error("There is no failed VN input to retry.");
    const input = this.pendingInput;
    let current: VnViewV1;
    try { current = await this.options.reader.inspect(); }
    catch (error) {
      if (isUnknownSaveOutcome(error)) this.publish("save-unknown", this.screen, this.target, { kind: "save-unknown", retryable: true }, "unknown");
      else this.publish("reload-required", this.screen, this.target, { kind: "reload-required", retryable: false }, "unknown");
      return;
    }
    if (current.view.cursor !== input.expected) {
      this.pendingInput = undefined;
      await this.prepare(current, "restore");
      return;
    }
    await this.submit(input, true);
  }

  /** Repeats only media/text preparation for the already-confirmed current target. */
  async retryResources(): Promise<void> {
    this.assertOpen();
    if (this.snapshot.status !== "unavailable" || !this.target) throw new Error("There is no failed VN presentation to retry.");
    await this.prepare(this.target, this.pendingEntry);
  }

  /** Re-resolves only a changed host presentation variant without changing the reader cursor. */
  async refreshPresentation(): Promise<void> {
    if (this.closed || this.snapshot.status !== "ready" || this.snapshot.saveStatus !== "saved" || !this.screen) return;
    const view = this.screen.view;
    const slots = mediaSlots(view);
    if (!slots.some(slot => !hasAsset(this.screen?.assets ?? [], slot, this.options.selectVariant(slot, view)))) return;
    await this.prepare(view, "refresh");
  }

  /** Retries current audio only; it never advances or changes the narrative cursor. */
  async retryAudio(): Promise<void> {
    this.assertOpen();
    const screen = this.screen;
    if (!screen || this.snapshot.status !== "ready") throw new Error("VN audio can be retried only on a ready screen.");
    const generation = this.generation;
    this.presentationGeneration++;
    this.audioLifecycle.abort();
    this.audioLifecycle = new AbortController();
    this.abortOperation();
    const operation = new AbortController();
    this.operation = operation;
    this.audioLoading = true;
    this.publish("ready", screen, undefined, undefined, "saved");
    const audioSlots = mediaSlots(screen.view).filter(slot => slot.role === "music" || slot.role === "voice");
    try {
      const result = await this.resolveAssets(audioSlots, screen.view, operation.signal, screen, visualAssets(screen));
      if (!this.isOperationCurrent(generation, operation)) {
        releaseAssets(result.assets, screen.assets);
        return;
      }
      if (this.screen !== screen) {
        releaseAssets(result.assets, this.screen?.assets ?? []);
        this.audioLoading = false;
        if (this.screen) this.publish("ready", this.screen, undefined, undefined, this.snapshot.saveStatus);
        return;
      }
      const nextAssets = [...screen.assets.filter(asset => asset.slot.role !== "music" && asset.slot.role !== "voice"), ...result.assets];
      const nextScreen: VnPresentedView = { ...screen, assets: nextAssets, audioIssues: result.audioIssues };
      this.screen = nextScreen;
      releaseAssets(screen.assets, nextAssets);
      if (this.soundStatus === "on") {
        const newlyPrepared = result.assets.filter(asset => !screen.assets.some(current => current.presentation === asset.presentation));
        const playRoles = new Set(newlyPrepared.map(asset => asset.slot.role));
        const playback = await this.playAudioForScreen(nextScreen, "audio-retry", screen, this.audioLifecycle.signal, playRoles);
        if (this.isOperationCurrent(generation, operation)) {
          this.soundStatus = playback ? "on" : result.audioIssues.length > 0 ? "unavailable" : "off";
        }
      }
      if (!this.isOperationCurrent(generation, operation)) return;
      this.audioLoading = false;
      this.publish("ready", nextScreen, undefined, undefined, "saved");
    } catch (error) {
      if (!this.isOperationCurrent(generation, operation)) return;
      this.audioLoading = false;
      if (isAbortError(error)) return;
      const failedScreen: VnPresentedView = { ...screen, audioIssues: uniqueRoles([...screen.audioIssues, ...audioSlots.map(slot => slot.role)]) };
      this.screen = failedScreen;
      this.publish("ready", failedScreen, undefined, undefined, "saved");
    }
  }

  async setSoundEnabled(enabled: boolean): Promise<void> {
    this.assertOpen();
    const screen = this.screen;
    if (!screen || (this.snapshot.status !== "ready" && this.snapshot.status !== "preparing")) return;
    if (!enabled) {
      this.presentationGeneration++;
      this.audioLifecycle.abort();
      this.audioLifecycle = new AbortController();
      stopAudio([...screen.assets, ...this.pendingAssets]);
      this.soundStatus = "off";
      this.publish(this.snapshot.status, screen, this.target, this.snapshot.failure, this.snapshot.saveStatus);
      return;
    }
    if (this.snapshot.status !== "ready") return;
    this.presentationGeneration++;
    this.audioLifecycle.abort();
    this.audioLifecycle = new AbortController();
    const generation = this.presentationGeneration;
    this.soundStatus = "on";
    const played = await this.playAudioForScreen(screen, "restore", undefined, this.audioLifecycle.signal);
    if (generation !== this.presentationGeneration || this.closed) return;
    this.soundStatus = played || this.audioLoading ? "on" : screen.audioIssues.length > 0 || hasPlayableAudio(screen) ? "unavailable" : "off";
    this.publish("ready", screen, undefined, undefined, "saved");
  }

  async retrySave(): Promise<void> {
    this.assertOpen();
    if (this.snapshot.status !== "save-unknown") throw new Error("There is no unknown VN save to retry.");
    this.publish("save-unknown", this.screen, this.target, this.snapshot.failure, "saving");
    try {
      const retried = await this.options.reader.retrySave();
      const view = retried ? await this.options.reader.inspect() : await this.reloadAndInspect();
      this.saveOutcomeUnknown = false;
      this.pendingInput = undefined;
      this.pendingCheckout = undefined;
      this.pendingEntry = "restore";
      this.stopScreenAudio(this.screen);
      await this.prepare(view, "restore");
    } catch (error) {
      if (isUnknownSaveOutcome(error)) {
        this.saveOutcomeUnknown = true;
        this.publish("save-unknown", this.screen, this.target, { kind: "save-unknown", retryable: true }, "unknown");
        return;
      }
      try {
        const view = await this.reloadAndInspect();
        this.saveOutcomeUnknown = false;
        this.pendingInput = undefined;
        this.pendingCheckout = undefined;
        this.pendingEntry = "restore";
        this.stopScreenAudio(this.screen);
        await this.prepare(view, "restore");
      } catch (reloadError) {
        if (isUnknownSaveOutcome(reloadError)) {
          this.saveOutcomeUnknown = true;
          this.publish("save-unknown", this.screen, this.target, { kind: "save-unknown", retryable: true }, "unknown");
        } else {
          this.publish("reload-required", this.screen, this.target, { kind: "reload-required", retryable: false }, "unknown");
        }
      }
    }
  }

  async reloadProgress(): Promise<void> {
    this.assertOpen();
    this.publish("reload-required", this.screen, this.target, { kind: "reload-required", retryable: false }, "saving");
    try {
      const view = await this.reloadAndInspect();
      this.saveOutcomeUnknown = false;
      this.pendingInput = undefined;
      this.pendingCheckout = undefined;
      this.stopScreenAudio(this.screen);
      await this.prepare(view, "restore");
    } catch (error) {
      if (isUnknownSaveOutcome(error)) {
        this.saveOutcomeUnknown = true;
        this.publish("save-unknown", this.screen, this.target, { kind: "save-unknown", retryable: true }, "unknown");
      }
      else this.publish("reload-required", this.screen, this.target, { kind: "reload-required", retryable: false }, "unknown");
    }
  }

  async checkout(commit: CommitId): Promise<void> {
    const screen = this.requireInteractiveScreen();
    if (!screen.view.view.history.some(entry => entry.id === commit)) throw new Error("The requested VN history entry is not in the current view.");
    await this.performCheckout(commit, screen);
  }

  private async performCheckout(commit: CommitId, screen: VnPresentedView): Promise<void> {
    const generation = this.generation;
    this.pendingCheckout = commit;
    this.publish("submitting", screen, undefined, undefined, "saving");
    try {
      await this.runLogical(() => this.options.reader.checkout(commit), error => {
        if (isUnknownSaveOutcome(error)) {
          this.saveOutcomeUnknown = true;
          this.pendingCheckout = commit;
        }
      });
      const view = await this.options.reader.inspect();
      if (!this.isCurrent(generation)) return;
      this.pendingInput = undefined;
      this.pendingCheckout = undefined;
      this.saveOutcomeUnknown = false;
      this.stopScreenAudio(screen);
      this.pendingEntry = "checkout";
      await this.prepare(view, "checkout");
    } catch (error) {
      if (isUnknownSaveOutcome(error)) {
        this.saveOutcomeUnknown = true;
        this.pendingCheckout = commit;
      }
      if (!this.isCurrent(generation)) return;
      await this.handleCheckoutFailure(commit, screen, error, generation);
    }
  }

  async back(): Promise<void> {
    const screen = this.requireInteractiveScreen();
    const previous = screen.view.view.history
      .filter(entry => entry.id !== screen.view.view.cursor && entry.depth < screen.view.view.depth)
      .sort((left, right) => right.depth - left.depth)[0];
    if (previous) await this.checkout(previous.id);
  }

  /** Clears all account-scoped bytes and handles synchronously before resolving the new context. */
  async setContext(context: VnPlayerContext<HostContext>): Promise<void> {
    this.assertOpen();
    const previousStatus = this.snapshot.status;
    const previousFailure = this.snapshot.failure;
    const previousSaveStatus = this.snapshot.saveStatus;
    const logicalWork = this.logicalWork;
    this.generation++;
    this.presentationGeneration++;
    this.context = context;
    this.audioLifecycle.abort();
    this.audioLifecycle = new AbortController();
    this.abortOperation();
    this.releasePendingAssets(this.screen?.assets ?? []);
    this.stopScreenAudio(this.screen);
    releaseAssets(this.screen?.assets ?? []);
    this.screen = undefined;
    this.target = undefined;
    this.audioLoading = false;
    if (previousSaveStatus !== "unknown" && previousStatus !== "save-unknown" && previousStatus !== "reload-required" && previousStatus !== "submitting") {
      this.pendingInput = undefined;
      this.pendingCheckout = undefined;
    }
    this.soundStatus = "off";
    this.publish("loading", undefined, undefined, undefined, this.snapshot.saveStatus);
    const generation = this.generation;
    if (previousStatus === "save-unknown" || previousStatus === "reload-required" || previousSaveStatus === "unknown") {
      if (previousStatus === "reload-required" || previousFailure?.kind === "reload-required") {
        this.publish("reload-required", undefined, undefined, { kind: "reload-required", retryable: false }, "unknown");
      } else {
        this.publish("save-unknown", undefined, undefined, { kind: "save-unknown", retryable: true }, "unknown");
      }
      return;
    }
    if (logicalWork) await logicalWork.catch(() => undefined);
    if (!this.isCurrent(generation)) return;
    if (this.saveOutcomeUnknown) {
      this.publish("save-unknown", undefined, undefined, { kind: "save-unknown", retryable: true }, "unknown");
      return;
    }
    this.pendingInput = undefined;
    this.pendingCheckout = undefined;
    try {
      const view = await this.options.reader.inspect();
      if (!this.isCurrent(generation)) return;
      await this.prepare(view, "context");
    } catch (error) {
      if (!this.isCurrent(generation)) return;
      if (isUnknownSaveOutcome(error)) {
        this.saveOutcomeUnknown = true;
        this.publish("save-unknown", undefined, undefined, { kind: "save-unknown", retryable: true }, "unknown");
      }
      else this.publish("unavailable", undefined, undefined, { kind: "resource", issue: "unavailable", retryable: true });
    }
  }

  async close(): Promise<void> {
    if (this.closed) return;
    this.closed = true;
    this.generation++;
    this.presentationGeneration++;
    this.audioLifecycle.abort();
    this.abortOperation();
    const finalSaveStatus = this.snapshot.saveStatus === "saving" ? "unknown" : this.snapshot.saveStatus;
    const owned = [...(this.screen?.assets ?? []), ...this.pendingAssets];
    stopAudio(owned);
    releaseAssets(owned);
    this.pendingAssets.clear();
    this.audioLoading = false;
    this.screen = undefined;
    this.target = undefined;
    this.soundStatus = "off";
    this.publish("closed", undefined, undefined, undefined, finalSaveStatus);
  }

  private async submit(input: PendingInput, retry = false): Promise<void> {
    await this.performSubmit(input, retry);
  }

  private async performSubmit(input: PendingInput, retry: boolean): Promise<void> {
    this.assertOpen();
    const screen = this.screen;
    if (!screen || (!retry && this.snapshot.status !== "ready")) throw new Error("VN input is disabled until the current screen is ready.");
    if (this.snapshot.status === "submitting" || this.snapshot.status === "preparing") throw new Error("A VN transition is already in progress.");
    const generation = this.generation;
    this.saveOutcomeUnknown = false;
    this.pendingInput = input;
    this.publish("submitting", screen, undefined, undefined, "saving");
    try {
      const confirmed = await this.runLogical(() => input.action === "advance"
        ? this.options.reader.advance(input.expected, readDialogue(input.interaction))
        : this.options.reader.choose(input.expected, readChoicePoint(input.interaction), input.options ?? []), error => {
          if (isUnknownSaveOutcome(error)) {
            this.saveOutcomeUnknown = true;
            this.pendingInput = input;
          }
        });
      if (!this.isCurrent(generation)) return;
      const view = await this.options.reader.inspect();
      if (!this.isCurrent(generation)) return;
      if (view.view.cursor !== confirmed) throw new Error("The VN reader returned a different confirmed cursor.");
      this.pendingInput = undefined;
      this.saveOutcomeUnknown = false;
      this.pendingEntry = "forward";
      this.onForwardCommit(screen, view);
      await this.prepare(view, "forward");
    } catch (error) {
      if (isUnknownSaveOutcome(error)) {
        this.saveOutcomeUnknown = true;
        this.pendingInput = input;
      }
      if (!this.isCurrent(generation)) return;
      await this.handleInputFailure(input, error, generation);
    }
  }

  private async handleInputFailure(input: PendingInput, error: unknown, generation: number): Promise<void> {
    if (isUnknownSaveOutcome(error)) {
      this.saveOutcomeUnknown = true;
      this.pendingInput = input;
      this.publish("save-unknown", this.screen, this.target, { kind: "save-unknown", retryable: true }, "unknown");
      return;
    }
    try {
      const view = await this.options.reader.inspect();
      if (!this.isCurrent(generation)) return;
      if (view.view.cursor !== input.expected) {
        this.pendingInput = undefined;
        this.stopScreenAudio(this.screen);
        await this.prepare(view, "restore");
        return;
      }
      this.pendingInput = input;
      this.publish("logical-error", this.screen, view, { kind: "logical", action: input.action, retryable: true }, "saved");
    } catch (inspectError) {
      if (!this.isCurrent(generation)) return;
      if (isUnknownSaveOutcome(inspectError)) {
        this.pendingInput = input;
        this.publish("save-unknown", this.screen, this.target, { kind: "save-unknown", retryable: true }, "unknown");
      } else {
        this.pendingInput = undefined;
        this.publish("reload-required", this.screen, this.target, { kind: "reload-required", retryable: false }, "unknown");
      }
    }
  }

  private async retryCheckout(commit: CommitId): Promise<void> {
    const screen = this.screen;
    if (!screen) throw new Error("The current VN screen is unavailable.");
    await this.performCheckout(commit, screen);
  }

  private async handleCheckoutFailure(commit: CommitId, screen: VnPresentedView, error: unknown, generation: number): Promise<void> {
    if (isUnknownSaveOutcome(error)) {
      this.saveOutcomeUnknown = true;
      this.pendingCheckout = commit;
      this.publish("save-unknown", screen, this.target, { kind: "save-unknown", retryable: true }, "unknown");
      return;
    }
    try {
      const view = await this.options.reader.inspect();
      if (!this.isCurrent(generation)) return;
      if (view.view.cursor !== screen.view.view.cursor) {
        this.pendingCheckout = undefined;
        this.stopScreenAudio(screen);
        await this.prepare(view, "restore");
        return;
      }
      this.pendingCheckout = commit;
      this.publish("logical-error", screen, view, { kind: "logical", action: "checkout", retryable: true }, "saved");
    } catch (inspectError) {
      if (!this.isCurrent(generation)) return;
      if (isUnknownSaveOutcome(inspectError)) {
        this.pendingCheckout = commit;
        this.publish("save-unknown", screen, this.target, { kind: "save-unknown", retryable: true }, "unknown");
      } else this.publish("reload-required", screen, this.target, { kind: "reload-required", retryable: false }, "unknown");
    }
  }

  private async prepare(view: VnViewV1, entry: VnEntryReason): Promise<void> {
    this.assertOpen();
    this.abortOperation();
    this.releasePendingAssets(this.screen?.assets ?? []);
    this.presentationGeneration++;
    this.audioLifecycle.abort();
    this.audioLifecycle = new AbortController();
    const controller = new AbortController();
    this.operation = controller;
    const generation = this.generation;
    let prior = this.screen;
    this.audioLoading = false;
    this.target = view;
    this.pendingEntry = entry;
    if (entry === "refresh") this.publish("ready", prior, undefined, undefined, "saved");
    else this.publish("preparing", prior, view, undefined, "saved");
    let slots: VnMediaSlot[];
    try {
      slots = mediaSlots(view);
      if (slots.length > this.limits.maxRequests) throw new VnMediaBoundaryError("budget");
    } catch (error) {
      const issue = error instanceof VnMediaBoundaryError && error.issue === "budget" ? "budget" : "invalid";
      this.publish("unavailable", prior, view, { kind: "resource", issue, retryable: true }, "saved");
      return;
    }
    const visualSlots = slots.filter(slot => slot.role === "background" || slot.role === "character");
    const audioSlots = slots.filter(slot => slot.role === "music" || slot.role === "voice");
    const textPromise = resolveVnText(view, this.options.text, this.context.content, controller.signal);
    const visualPromise = this.resolveAssets(visualSlots, view, controller.signal, prior);
    let texts: ReadonlyMap<string, string>;
    let visuals: WorkResult;
    try {
      [texts, visuals] = await Promise.all([textPromise, visualPromise]);
    } catch (error) {
      controller.abort();
      const settled = await Promise.allSettled([textPromise, visualPromise]);
      const completedVisuals = settled[1];
      if (completedVisuals?.status === "fulfilled") releaseAssets(completedVisuals.value.assets, prior?.assets);
      if (!this.isOperationCurrent(generation, controller)) return;
      this.target = view;
      const issue = error instanceof VnMediaBoundaryError
        ? error.issue === "budget" ? "budget" : "invalid"
        : error instanceof VnTextUnavailableError ? "text" : "unavailable";
      this.publish("unavailable", prior, view, { kind: "resource", issue, retryable: true }, "saved");
      return;
    }

    if (!this.isOperationCurrent(generation, controller) || controller.signal.aborted) {
      releaseAssets(visuals.assets, prior?.assets);
      return;
    }
    prior = this.screen;
    const reusableAudio = reusableAssets(audioSlots, view, prior, this.options.selectVariant);
    let nextAssets = [...visuals.assets, ...reusableAudio];
    let audioIssues = [...visuals.audioIssues];
    if (exceedsMediaBudget(nextAssets, [...(prior?.assets ?? []), ...nextAssets], this.limits)) {
      const droppedAudio = nextAssets.filter(asset => isAudioRole(asset.slot.role));
      if (droppedAudio.length > 0) {
        const roles = uniqueRoles(droppedAudio.map(asset => asset.slot.role));
        nextAssets = nextAssets.filter(asset => !isAudioRole(asset.slot.role));
        audioIssues = uniqueRoles([...audioIssues, ...roles]);
        if (prior) {
          this.dropOldAudio(prior, roles);
          prior = this.screen;
        }
      }
    }
    if (exceedsMediaBudget(nextAssets, [...(prior?.assets ?? []), ...nextAssets], this.limits)) {
      releaseAssets(visuals.assets, prior?.assets ?? []);
      this.target = view;
      this.publish("unavailable", prior, view, { kind: "resource", issue: "budget", retryable: true }, "saved");
      return;
    }
    const next: VnPresentedView = {
      view,
      texts,
      assets: nextAssets,
      audioIssues,
      entry,
    };
    this.pendingAssets = new Set(next.assets);
    try {
      await this.playAudioForScreen(next, entry, prior, this.audioLifecycle.signal);
      if (!this.isOperationCurrent(generation, controller) || controller.signal.aborted) {
        this.releasePendingAssets(prior?.assets ?? []);
        return;
      }
      this.screen = next;
      this.pendingAssets.clear();
      this.target = undefined;
      this.pendingInput = undefined;
      this.pendingCheckout = undefined;
      releaseAssets(prior?.assets ?? [], next.assets);
      const missingAudio = audioSlots.some(slot => !hasAsset(next.assets, slot, this.options.selectVariant(slot, view)));
      this.audioLoading = missingAudio;
      if (this.soundStatus === "on" && !hasPlayableAudio(next) && !missingAudio && next.audioIssues.length > 0) this.soundStatus = "unavailable";
      this.publish("ready", next, undefined, undefined, "saved");
      if (missingAudio) void this.prepareAudio(next, audioSlots, entry, generation, controller);
    } catch (error) {
      this.releasePendingAssets(prior?.assets ?? []);
      if (!this.isOperationCurrent(generation, controller)) return;
      this.target = view;
      const issue = error instanceof VnMediaBoundaryError ? (error.issue === "budget" ? "budget" : "invalid") : "unavailable";
      this.publish("unavailable", prior, view, { kind: "resource", issue, retryable: true }, "saved");
    }
  }

  private async prepareAudio(
    screen: VnPresentedView,
    slots: readonly VnMediaSlot[],
    entry: VnEntryReason,
    generation: number,
    operation: AbortController,
  ): Promise<void> {
    let resolvedAssets: readonly VnPreparedAsset[] = [];
    try {
      const result = await this.resolveAssets(slots, screen.view, operation.signal, screen, visualAssets(screen));
      resolvedAssets = result.assets;
      if (!this.isOperationCurrent(generation, operation) || operation.signal.aborted) {
        releaseAssets(result.assets, screen.assets);
        return;
      }
      if (this.screen !== screen) {
        releaseAssets(result.assets, this.screen?.assets ?? []);
        this.audioLoading = false;
        if (this.screen) this.publish("ready", this.screen, undefined, undefined, this.snapshot.saveStatus);
        return;
      }
      const nextAssets = [...screen.assets.filter(asset => asset.slot.role !== "music" && asset.slot.role !== "voice"), ...result.assets];
      const next: VnPresentedView = { ...screen, assets: nextAssets, audioIssues: result.audioIssues };
      assertRetainedBudget(next.assets, [...screen.assets, ...next.assets], this.limits);
      this.pendingAssets = new Set(next.assets);
      const newlyPreparedAudio = result.assets.filter(asset => !screen.assets.some(current => current.presentation === asset.presentation));
      const playRoles = new Set(newlyPreparedAudio.map(asset => asset.slot.role));
      await this.playAudioForScreen(next, entry, screen, this.audioLifecycle.signal, playRoles);
      if (!this.isOperationCurrent(generation, operation) || operation.signal.aborted || this.screen !== screen) {
        this.releasePendingAssets(screen.assets);
        return;
      }
      this.screen = next;
      this.pendingAssets.clear();
      this.audioLoading = false;
      releaseAssets(screen.assets, next.assets);
      if (this.soundStatus === "on" && !hasPlayableAudio(next) && next.audioIssues.length > 0) this.soundStatus = "unavailable";
      this.publish("ready", next, undefined, undefined, this.snapshot.saveStatus);
    } catch (error) {
      if (!this.isOperationCurrent(generation, operation) || operation.signal.aborted || this.screen !== screen) return;
      releaseAssets(resolvedAssets, screen.assets);
      this.audioLoading = false;
      if (isAbortError(error)) return;
      const roles = slots.map(slot => slot.role);
      const failed: VnPresentedView = { ...screen, audioIssues: uniqueRoles([...screen.audioIssues, ...roles]) };
      this.screen = failed;
      this.publish("ready", failed, undefined, undefined, this.snapshot.saveStatus);
    }
  }

  private async resolveAssets(
    slots: readonly VnMediaSlot[],
    view: VnViewV1,
    parentSignal: AbortSignal,
    prior?: VnPresentedView,
    targetBase: readonly VnPreparedAsset[] = [],
  ): Promise<WorkResult> {
    if (slots.length > this.limits.maxRequests) throw new VnMediaBoundaryError("budget");
    const controller = new AbortController();
    const abortChild = () => controller.abort();
    const oldAssets = prior?.assets ?? [];
    const work: Array<{ slot: VnMediaSlot; variant: string }> = [];
    let assets: VnPreparedAsset[] = [];
    const issues = new Set<VnMediaRole>();
    for (const slot of slots) {
      const variant = this.options.selectVariant(slot, view);
      if (!variantId(variant)) throw new VnMediaBoundaryError("variant");
      const reusable = oldAssets.find(asset => asset.variant === variant && asset.slot.role === slot.role && sameMedia(asset.media, slot.media));
      if (reusable) assets.push({ ...reusable, slot });
      else work.push({ slot, variant });
    }
    if (work.length > this.limits.maxRequests) throw new VnMediaBoundaryError("budget");
    const overTarget = (candidate: readonly VnPreparedAsset[]) =>
      encodedBytes([...targetBase, ...candidate]) > this.limits.maxEncodedBytesPerTarget
      || decodedBytes([...targetBase, ...candidate]) > this.limits.maxDecodedBytesRetained;
    if (overTarget(assets)) {
      const removable = assets.filter(asset => asset.slot.role === "music" || asset.slot.role === "voice");
      assets = assets.filter(asset => asset.slot.role !== "music" && asset.slot.role !== "voice");
      for (const asset of removable) issues.add(asset.slot.role);
      if (prior && removable.length > 0) this.dropOldAudio(prior, removable.map(asset => asset.slot.role));
      if (overTarget(assets)) throw new VnMediaBoundaryError("budget");
    }
    let retainedOld = uniqueAssets(oldAssets);
    const abortParent = () => controller.abort();
    if (parentSignal.aborted) controller.abort();
    else parentSignal.addEventListener("abort", abortParent, { once: true });
    const prepared: VnPreparedAsset[] = [];
    let next = 0;
    let failure: unknown;
    const worker = async () => {
      while (!controller.signal.aborted) {
        const index = next++;
        if (index >= work.length) return;
        const item = work[index];
        if (!item) return;
        try {
          const asset = await this.resolveOne(item.slot, item.variant, view, controller.signal);
          if (controller.signal.aborted) {
            if (asset) releaseAssets([asset]);
            return;
          }
          if (asset) {
            const targetCandidate = [...assets, ...prepared, asset];
            const retainedCandidate = [...retainedOld, ...prepared, asset];
            const targetOver = overTarget(targetCandidate);
            const retainedOver = encodedBytes(retainedCandidate) > this.limits.maxEncodedBytesRetained
              || decodedBytes(retainedCandidate) > this.limits.maxDecodedBytesRetained;
            if (targetOver || retainedOver) {
              releaseAssets([asset]);
              if (item.slot.role === "music" || item.slot.role === "voice") {
                issues.add(item.slot.role);
                continue;
              }
              if (retainedOver) {
                const oldAudio = retainedOld.filter(candidate => candidate.slot.role === "music" || candidate.slot.role === "voice");
                if (prior && oldAudio.length > 0) {
                  this.dropOldAudio(prior, oldAudio.map(candidate => candidate.slot.role));
                  retainedOld = retainedOld.filter(candidate => candidate.slot.role !== "music" && candidate.slot.role !== "voice");
                  assets = assets.filter(candidate => candidate.slot.role !== "music" && candidate.slot.role !== "voice");
                  for (const old of oldAudio) issues.add(old.slot.role);
                  if (!overTarget([...assets, ...prepared, asset])
                    && encodedBytes([...retainedOld, ...prepared, asset]) <= this.limits.maxEncodedBytesRetained
                    && decodedBytes([...retainedOld, ...prepared, asset]) <= this.limits.maxDecodedBytesRetained) {
                    prepared.push(asset);
                    continue;
                  }
                }
              }
              throw new VnMediaBoundaryError("budget");
            }
            prepared.push(asset);
          } else if (item.slot.role === "music" || item.slot.role === "voice") issues.add(item.slot.role);
          else throw new VnMediaUnavailableError(item.slot.role);
        } catch (error) {
          if (controller.signal.aborted && parentSignal.aborted) return;
          if (item.slot.role === "music" || item.slot.role === "voice") {
            issues.add(item.slot.role);
            continue;
          }
          failure ??= error;
          controller.abort();
          return;
        }
      }
    };
    try {
      await Promise.all(Array.from({ length: Math.min(this.limits.maxConcurrent, work.length) }, () => worker()));
      if (parentSignal.aborted) throw abortError();
      if (failure !== undefined) throw failure;
      if (overTarget([...assets, ...prepared]) || encodedBytes([...retainedOld, ...prepared]) > this.limits.maxEncodedBytesRetained
        || decodedBytes([...retainedOld, ...prepared]) > this.limits.maxDecodedBytesRetained) throw new VnMediaBoundaryError("budget");
      return { assets: [...assets, ...prepared], audioIssues: [...issues] };
    } catch (error) {
      releaseAssets(prepared, oldAssets);
      throw error;
    } finally {
      parentSignal.removeEventListener("abort", abortParent);
    }
  }

  private async resolveOne(slot: VnMediaSlot, variant: string, view: VnViewV1, signal: AbortSignal): Promise<VnPreparedAsset | undefined> {
    const expected = expectedKind(slot.role);
    if (slot.media.kind !== expected) throw new VnMediaBoundaryError("kind");
    if (slot.media.provider.length === 0 || slot.media.key.length === 0 || slot.media.revision.length === 0) throw new VnMediaBoundaryError("media");
    const media = Object.freeze({ ...slot.media });
    const frozenSlot = Object.freeze({ ...slot, media }) as VnMediaSlot;
    const request: VnMediaRequest<HostContext> = Object.freeze({
      requestId: `vn-media-${++this.requestSequence}`,
      execution: view.view.execution,
      commit: view.view.cursor,
      artifact: view.view.artifact_id,
      slot: frozenSlot,
      kind: expected,
      variant,
      context: this.context.media,
      signal,
    });
    if (signal.aborted) throw abortError();
    const raw = await awaitAbortable(this.options.resolveMedia(request), signal, value => {
      try { this.options.onLateDiscard?.(request); } catch { /* Telemetry must not block resource cleanup. */ }
      releaseVnMediaResolution(value);
    });
    if (signal.aborted) {
      try { this.options.onLateDiscard?.(request); } catch { /* Telemetry must not block resource cleanup. */ }
      releaseVnMediaResolution(raw);
      throw abortError();
    }
    let result: VnMediaResolution;
    try { result = checkedVnMediaResolution(request, raw, this.limits); }
    catch (error) {
      releaseVnMediaResolution(raw);
      throw error;
    }
    if (result.status === "unavailable") return undefined;
    let resourceReleased = false;
    const releaseResource = () => {
      if (resourceReleased) return;
      resourceReleased = true;
      result.release();
    };
    const safeResource = {
      ...result,
      media: Object.freeze({ ...result.media }),
      descriptor: Object.freeze({ ...result.descriptor }),
      bytes: new Uint8Array(result.bytes),
      release: releaseResource,
    } as typeof result;
    const decodedBytes = decodedMediaBytes(safeResource.descriptor);
    if (!Number.isSafeInteger(decodedBytes) || decodedBytes <= 0 || decodedBytes > this.limits.maxDecodedBytesRetained) {
      releaseResource();
      throw new VnMediaBoundaryError("budget");
    }
    let presentation: VnPresentedMedia | undefined;
    try {
      const presented = await awaitAbortable(
        this.options.presentMedia(safeResource, { slot: frozenSlot, variant, signal }, view),
        signal,
        value => {
          try { this.options.onLateDiscard?.(request); } catch { /* Telemetry must not block resource cleanup. */ }
          releasePresentedValue(value);
        },
      );
      presentation = ownPresentation(checkedPresentedMedia(presented, expected));
      releaseResource();
      if (signal.aborted) {
        try { this.options.onLateDiscard?.(request); } catch { /* Telemetry must not block resource cleanup. */ }
        safeRelease(presentation);
        throw abortError();
      }
      return { slot: frozenSlot, media, variant, presentation, encodedBytes: safeResource.bytes.byteLength, decodedBytes };
    } catch (error) {
      releaseResource();
      if (presentation) safeRelease(presentation);
      throw error;
    }
  }

  private async playAudioForScreen(
    next: VnPresentedView,
    entry: VnEntryReason,
    prior?: VnPresentedView,
    signal: AbortSignal = this.audioLifecycle.signal,
    playRoles?: ReadonlySet<VnMediaRole>,
  ): Promise<boolean> {
    if (signal.aborted) throw abortError();
    const previousMusic = prior?.assets.find(asset => asset.slot.role === "music")?.presentation;
    const nextMusic = next.assets.find(asset => asset.slot.role === "music")?.presentation;
    const previousVoice = prior?.assets.find(asset => asset.slot.role === "voice")?.presentation;
    const nextVoice = next.assets.find(asset => asset.slot.role === "voice")?.presentation;
    const isForward = entry === "forward";
    const action = next.view.view.music_action;

    if (entry === "refresh") return this.soundStatus === "on" && hasPlayableAudio(next);

    // onForwardCommit stops the departing line and music before async forward presentation work.
    if (!isForward && (entry === "checkout" || entry === "restore" || entry === "context")) {
      stopIfAudio(previousVoice);
      stopIfAudio(previousMusic);
    }
    if (this.soundStatus !== "on") return false;

    let attempted = false;
    let played = false;
    const shouldPlayMusic = nextMusic?.kind === "audio"
      && (!playRoles || playRoles.has("music"))
      && (!isForward || action?.kind === "play" || (action?.kind === "keep" && !sameAudioTarget(prior, next)));
    if (shouldPlayMusic && nextMusic?.kind === "audio") {
      attempted = true;
      try { await nextMusic.playFromStart(signal); played = true; }
      catch (error) { if (signal.aborted) throw abortError(); stopIfAudio(nextMusic); }
    }
    const shouldPlayVoice = nextVoice?.kind === "audio" && (!playRoles || playRoles.has("voice"));
    if (shouldPlayVoice && nextVoice?.kind === "audio") {
      attempted = true;
      try { await nextVoice.playFromStart(signal); played = true; }
      catch (error) { if (signal.aborted) throw abortError(); stopIfAudio(nextVoice); }
    }
    if (attempted && !played) {
      this.soundStatus = "unavailable";
      return false;
    }
    return played || (!attempted && this.soundStatus === "on" && hasPlayableAudio(next));
  }

  private onForwardCommit(prior: VnPresentedView, next: VnViewV1): void {
    stopAudio(prior.assets.filter(asset => asset.slot.role === "voice"));
    const action = next.view.music_action;
    const previousMusic = prior.assets.find(asset => asset.slot.role === "music");
    const nextMusic = next.view.scene.music;
    const sameTarget = previousMusic !== undefined && nextMusic != null
      && previousMusic.variant === this.options.selectVariant({ role: "music", id: "music", media: nextMusic }, next)
      && sameMedia(previousMusic.media, nextMusic);
    if (action?.kind === "play" || action?.kind === "stop" || !sameTarget) {
      stopAudio(prior.assets.filter(asset => asset.slot.role === "music"));
    }
  }

  private requireInteractiveScreen(): VnPresentedView {
    this.assertOpen();
    if (this.snapshot.status !== "ready" || !this.screen) throw new Error("VN input is disabled until the current screen is ready.");
    return this.screen;
  }

  private async reloadAndInspect(): Promise<VnViewV1> {
    await this.options.reader.reload();
    return this.options.reader.inspect();
  }

  private stopScreenAudio(screen: VnPresentedView | undefined): void {
    if (screen) stopAudio(screen.assets);
  }

  private abortOperation(): void {
    this.operation?.abort();
    this.operation = undefined;
  }

  private isOperationCurrent(generation: number, operation: AbortController): boolean {
    return this.isCurrent(generation) && this.operation === operation;
  }

  private async runLogical<T>(operation: () => Promise<T>, onError?: (error: unknown) => void): Promise<T> {
    const work = operation().catch(error => {
      onError?.(error);
      throw error;
    });
    const tracked = work.then(() => undefined, () => undefined);
    this.logicalWork = tracked;
    try {
      return await work;
    } finally {
      if (this.logicalWork === tracked) this.logicalWork = undefined;
    }
  }

  private releasePendingAssets(keep: readonly VnPreparedAsset[] = []): void {
    const pending = [...this.pendingAssets];
    this.pendingAssets.clear();
    releaseAssets(pending, keep);
  }

  private dropOldAudio(prior: VnPresentedView, roles: readonly VnMediaRole[]): void {
    const removeRoles = new Set(roles);
    const removed = prior.assets.filter(asset => removeRoles.has(asset.slot.role));
    if (removed.length === 0) return;
    stopAudio(removed);
    releaseAssets(removed);
    const removedPresentations = new Set(removed.map(asset => asset.presentation));
    for (const pending of this.pendingAssets) {
      if (removedPresentations.has(pending.presentation)) this.pendingAssets.delete(pending);
    }
    const next: VnPresentedView = {
      ...prior,
      assets: prior.assets.filter(asset => !removeRoles.has(asset.slot.role)),
      audioIssues: uniqueRoles([...prior.audioIssues, ...roles]),
    };
    if (this.screen === prior) {
      this.screen = next;
      this.soundStatus = "unavailable";
      this.publish(this.snapshot.status, next, this.target, this.snapshot.failure, this.snapshot.saveStatus);
    }
  }

  private isCurrent(generation: number): boolean {
    return !this.closed && this.generation === generation;
  }

  private publish(
    status: VnPlayerStatus,
    screen: VnPresentedView | undefined,
    target: VnViewV1 | undefined,
    failure?: VnPlayerFailure,
    saveStatus: VnSaveStatus = this.snapshot.saveStatus,
  ): void {
    this.snapshot = {
      status,
      saveStatus,
      soundStatus: this.soundStatus,
      audioLoading: this.audioLoading,
      ...(screen ? { screen } : {}),
      ...(target ? { target } : {}),
      ...(failure ? { failure } : {}),
    };
    for (const listener of this.listeners) listener();
  }

  private assertOpen(): void {
    if (this.closed) throw new Error("The VN player is closed.");
  }
}

export function createVnPlayer<HostContext>(options: VnPlayerOptions<HostContext>): VnPlayerController<HostContext> {
  return new VnPlayerController(options);
}

export function readVnText(view: VnPresentedView, content: Content | null | undefined, args?: Record<string, ViewScalar>): string | undefined {
  if (!content) return undefined;
  const key = contentItemKey({ content, ...(args ? { args } : {}) });
  return view.texts.get(key);
}

function mediaSlots(view: VnViewV1): VnMediaSlot[] {
  const slots: VnMediaSlot[] = [];
  const scene = view.view.scene;
  if (scene.background) slots.push({ role: "background", id: "background", media: scene.background });
  const ids = new Set<string>();
  for (const layer of [...scene.layers].sort((left, right) => left.order - right.order || left.id.localeCompare(right.id))) {
    if (layer.id.length === 0 || ids.has(layer.id) || !Number.isSafeInteger(layer.order)) throw new VnMediaBoundaryError("descriptor");
    ids.add(layer.id);
    slots.push({ role: "character", id: layer.id, order: layer.order, media: layer.asset });
  }
  if (scene.music) slots.push({ role: "music", id: "music", media: scene.music });
  const interaction = view.view.interaction;
  if (interaction.kind === "advance" && interaction.voice) slots.push({ role: "voice", id: "voice", media: interaction.voice });
  return slots;
}

function readDialogue(interaction: VnInteractionV1): string {
  if (interaction.kind !== "advance") throw new Error("Expected a VN Advance interaction.");
  return interaction.dialogue;
}

function readChoicePoint(interaction: VnInteractionV1): string {
  if (interaction.kind !== "choose") throw new Error("Expected a VN Choose interaction.");
  return interaction.choice_point;
}

function releaseAssets(assets: readonly VnPreparedAsset[], keep: readonly VnPreparedAsset[] = []): void {
  const retained = new Set(keep.map(asset => asset.presentation));
  const released = new Set<VnPresentedMedia>();
  for (const asset of assets) {
    const presentation = asset.presentation;
    if (retained.has(presentation) || released.has(presentation)) continue;
    released.add(presentation);
    safeRelease(presentation);
  }
}

function stopAudio(assets: readonly VnPreparedAsset[]): void {
  const stopped = new Set<VnPresentedMedia>();
  for (const asset of assets) {
    const presentation = asset.presentation;
    if (presentation.kind !== "audio" || stopped.has(presentation)) continue;
    stopped.add(presentation);
    try { presentation.stop(); } catch { /* A released browser source can already be stopped. */ }
  }
}

function stopIfAudio(value: VnPresentedMedia | undefined): void {
  if (value?.kind === "audio") {
    try { value.stop(); } catch { /* A released browser source can already be stopped. */ }
  }
}

function safeRelease(value: VnPresentedMedia): void {
  try { value.release(); } catch { /* Host cleanup must not prevent other leases from releasing. */ }
}

function ownPresentation(value: VnPresentedMedia): VnPresentedMedia {
  let released = false;
  if (value.kind === "image") {
    return {
      kind: "image",
      source: value.source,
      release: () => {
        if (released) return;
        released = true;
        value.release();
      },
    };
  }
  let stopped = true;
  const stop = () => {
    if (released || stopped) return;
    stopped = true;
    value.stop();
  };
  const stopAfterLateStart = () => {
    stopped = true;
    try { value.stop(); } catch { /* An aborted play can finish after its handle was released. */ }
  };
  return {
    kind: "audio",
    playFromStart: async signal => {
      if (signal.aborted) throw abortError();
      if (released) throw new Error("The VN audio resource has been released.");
      stopped = false;
      try {
        await value.playFromStart(signal);
      } catch (error) {
        if (signal.aborted || released) {
          stopAfterLateStart();
          throw abortError();
        }
        throw error;
      }
      if (signal.aborted || released) {
        stopAfterLateStart();
        throw abortError();
      }
    },
    stop,
    release: () => {
      if (released) return;
      try { stop(); } finally {
        released = true;
        value.release();
      }
    },
  };
}

function sameAudioTarget(left: VnPresentedView | undefined, right: VnPresentedView): boolean {
  const a = left?.assets.find(asset => asset.slot.role === "music");
  const b = right.assets.find(asset => asset.slot.role === "music");
  return a !== undefined && b !== undefined && a.variant === b.variant && sameMedia(a.media, b.media);
}

function hasPlayableAudio(view: VnPresentedView): boolean {
  return view.assets.some(asset => asset.presentation.kind === "audio");
}

function hasAsset(assets: readonly VnPreparedAsset[], slot: VnMediaSlot, variant: string): boolean {
  return assets.some(asset => mediaSlotKey(asset.slot) === mediaSlotKey(slot)
    && asset.variant === variant && sameMedia(asset.media, slot.media));
}

function reusableAssets(
  slots: readonly VnMediaSlot[],
  view: VnViewV1,
  prior: VnPresentedView | undefined,
  selectVariant: (slot: VnMediaSlot, view: VnViewV1) => string,
): VnPreparedAsset[] {
  if (!prior) return [];
  const reusable: VnPreparedAsset[] = [];
  for (const slot of slots) {
    const variant = selectVariant(slot, view);
    const asset = prior.assets.find(candidate => mediaSlotKey(candidate.slot) === mediaSlotKey(slot)
      && candidate.variant === variant && sameMedia(candidate.media, slot.media));
    if (asset) reusable.push({ ...asset, slot });
  }
  return reusable;
}

function uniqueAssets(assets: readonly VnPreparedAsset[]): VnPreparedAsset[] {
  const seen = new Set<VnPresentedMedia>();
  return assets.filter(asset => {
    if (seen.has(asset.presentation)) return false;
    seen.add(asset.presentation);
    return true;
  });
}

function isAudioRole(role: VnMediaRole): boolean {
  return role === "music" || role === "voice";
}

function visualAssets(screen: VnPresentedView): readonly VnPreparedAsset[] {
  return screen.assets.filter(asset => !isAudioRole(asset.slot.role));
}

function encodedBytes(assets: readonly VnPreparedAsset[]): number {
  return uniqueAssets(assets).reduce((total, asset) => total + asset.encodedBytes, 0);
}

function decodedBytes(assets: readonly VnPreparedAsset[]): number {
  return uniqueAssets(assets).reduce((total, asset) => total + asset.decodedBytes, 0);
}

function assertRetainedBudget(
  current: readonly VnPreparedAsset[],
  retained: readonly VnPreparedAsset[],
  limits: VnMediaLimits,
): void {
  if (exceedsMediaBudget(current, retained, limits)) throw new VnMediaBoundaryError("budget");
}

function exceedsMediaBudget(
  current: readonly VnPreparedAsset[],
  retained: readonly VnPreparedAsset[],
  limits: VnMediaLimits,
): boolean {
  return encodedBytes(current) > limits.maxEncodedBytesPerTarget
    || encodedBytes(retained) > limits.maxEncodedBytesRetained
    || decodedBytes(retained) > limits.maxDecodedBytesRetained;
}

function uniqueRoles(roles: readonly VnMediaRole[]): VnMediaRole[] {
  return [...new Set(roles)];
}

function checkedPresentedMedia(value: unknown, expected: MediaKind): VnPresentedMedia {
  if (typeof value !== "object" || value === null || Array.isArray(value)) throw new VnMediaBoundaryError("descriptor");
  const result = value as Record<string, unknown>;
  if (result.kind !== expected) throw new VnMediaBoundaryError("kind");
  if (expected === "image") {
    if (typeof result.source !== "string" || result.source.length === 0 || typeof result.release !== "function") {
      throw new VnMediaBoundaryError("descriptor");
    }
    return value as VnPresentedMedia;
  }
  if (typeof result.playFromStart !== "function" || typeof result.stop !== "function" || typeof result.release !== "function") {
    throw new VnMediaBoundaryError("descriptor");
  }
  return value as VnPresentedMedia;
}

function releasePresentedValue(value: unknown): void {
  if (typeof value !== "object" || value === null || Array.isArray(value)) return;
  const result = value as Record<string, unknown>;
  if (typeof result.stop === "function") {
    try { (result.stop as () => void)(); } catch { /* A late host audio handle can already be stopped. */ }
  }
  if (typeof result.release === "function") {
    try { (result.release as () => void)(); } catch { /* A malformed late handle still gets its cleanup attempt. */ }
  }
}

function isAbortError(error: unknown): boolean {
  return error instanceof Error && error.name === "AbortError";
}

function isUnknownSaveOutcome(error: unknown): boolean {
  return error instanceof Error && /unknown outcome/i.test(error.message);
}

function awaitAbortable<T>(promise: Promise<T>, signal: AbortSignal, onLate: (value: T) => void): Promise<T> {
  return new Promise<T>((resolve, reject) => {
    let settled = false;
    const cleanup = () => signal.removeEventListener("abort", abort);
    const abort = () => {
      if (settled) return;
      settled = true;
      cleanup();
      reject(abortError());
    };
    if (signal.aborted) {
      settled = true;
      void promise.then(value => { try { onLate(value); } catch { /* Late cleanup is best effort. */ } }, () => undefined);
      reject(abortError());
      return;
    }
    signal.addEventListener("abort", abort, { once: true });
    promise.then(value => {
      if (settled || signal.aborted) {
        settled = true;
        cleanup();
        try { onLate(value); } catch { /* Late cleanup is best effort. */ }
        return;
      }
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

function positiveLimit(value: number): boolean {
  return Number.isSafeInteger(value) && value > 0;
}

function abortError(): DOMException {
  return new DOMException("VN presentation work was cancelled.", "AbortError");
}
