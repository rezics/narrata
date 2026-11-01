import "fake-indexeddb/auto";
import { expect, test, vi } from "vitest";
import { IndexedDbStore, StoreSuperseded, type CacheStore } from "@rezics/narrata/storage";
import { checkedVnMediaResolution, createVnPlayer, VnMediaBoundaryError, VnTextUnavailableError, type VnMediaPresenter, type VnMediaRequest, type VnMediaResolution, type VnMediaResolver, type VnPlayerContext, type VnTextResolver } from "@rezics/narrata/vn/player";
import { openVn } from "@rezics/narrata/vn";
import type { MediaRef, VnCandidatePreparation, VnDependencyLimits, VnDependencyResult, VnReader, VnViewV1 } from "@rezics/narrata/vn";
import type { VnPresentedMedia } from "@rezics/narrata/vn/player";
import { execution, lazyVnPack, objectId, splitPack, vnDependencyPack, vnPack } from "./fixture.js";

type TestContext = { readonly scope: string };

function textResolver(): VnTextResolver {
  return {
    resolve: async request => request.items.map(item => {
      const ref = "unit" in item.content ? item.content.unit : item.content;
      return { status: "ok", revision: "1", payload: { text: ref.key } };
    }),
  };
}

function mediaHarness(sizeForRequest: (request: VnMediaRequest<TestContext>) => number = () => 4) {
  const requests: VnMediaRequest<TestContext>[] = [];
  const hostReleases: string[] = [];
  const presentationReleases: string[] = [];
  const releasedPresentationIds: number[] = [];
  const presentations: Array<{ id: number; key: string; kind: "image" | "audio" }> = [];
  const audioEvents: Array<{ event: "play" | "stop" | "release"; id: number; key: string }> = [];
  const activeAudio = new Set<number>();
  const lateDiscards: string[] = [];
  let failKey: string | undefined;
  let lateResolve: ((result: VnMediaResolution) => void) | undefined;
  let lateRequest: VnMediaRequest<TestContext> | undefined;
  let lateKey: string | undefined;
  let presentationKey: string | undefined;
  let pendingPresentation: { id: number; key: string; kind: "image" | "audio"; resolve: (value: VnPresentedMedia) => void } | undefined;
  let playKey: string | undefined;
  let pendingPlay: { id: number; key: string; resolve: () => void } | undefined;
  let presentationId = 0;
  const resolveMedia: VnMediaResolver<TestContext> = async request => {
    requests.push(request);
    const media = request.slot.media;
    if (lateKey === media.key && request.context.scope === "reader-one") {
      lateKey = undefined;
      lateRequest = request;
      return new Promise(resolve => { lateResolve = resolve; });
    }
    if (failKey === media.key) {
      failKey = undefined;
      return { status: "unavailable", kind: request.kind, role: request.slot.role, media, revision: media.revision, variant: request.variant, reason: "temporary" };
    }
    return available(request, hostReleases, sizeForRequest(request));
  };
  const presentMedia: VnMediaPresenter = async (resource, request) => {
    const key = request.slot.media.key;
    const id = ++presentationId;
    presentations.push({ id, key, kind: resource.kind });
    if (presentationKey === key) {
      presentationKey = undefined;
      return await new Promise<VnPresentedMedia>(resolve => { pendingPresentation = { id, key, kind: resource.kind, resolve }; });
    }
    return createPresentation(resource.kind, key, id);
  };
  return {
    requests,
    hostReleases,
    presentationReleases,
    releasedPresentationIds,
    presentations,
    audioEvents,
    activeAudio,
    lateDiscards,
    resolveMedia,
    presentMedia,
    failOnce(key: string) { failKey = key; },
    delayOnce(key: string) { lateKey = key; },
    deferPresentationOnce(key: string) { presentationKey = key; },
    hasPendingPresentation() { return pendingPresentation !== undefined; },
    resolveDeferredPresentation() {
      const pending = pendingPresentation;
      if (!pending) throw new Error("No VN presentation is pending.");
      pendingPresentation = undefined;
      pending.resolve(createPresentation(pending.kind, pending.key, pending.id));
    },
    deferPlayOnce(key: string) { playKey = key; },
    hasPendingPlay() { return pendingPlay !== undefined; },
    resolveDeferredPlay() {
      const pending = pendingPlay;
      if (!pending) throw new Error("No VN audio play is pending.");
      pendingPlay = undefined;
      activeAudio.add(pending.id);
      audioEvents.push({ event: "play", id: pending.id, key: pending.key });
      pending.resolve();
    },
    releaseLate() {
      if (!lateResolve || !lateRequest) throw new Error("No late VN resource is pending.");
      lateResolve(available(lateRequest, hostReleases));
      lateResolve = undefined;
      lateRequest = undefined;
    },
  };

  function createPresentation(kind: "image" | "audio", key: string, id: number): VnPresentedMedia {
    let released = false;
    if (kind === "image") {
      return {
        kind: "image",
        source: `blob:${key}:${id}`,
        release: () => {
          if (released) return;
          released = true;
          presentationReleases.push(key);
          releasedPresentationIds.push(id);
        },
      };
    }
    return {
      kind: "audio",
      playFromStart: async () => {
        if (playKey === key) {
          playKey = undefined;
          await new Promise<void>(resolve => { pendingPlay = { id, key, resolve }; });
          return;
        }
        activeAudio.add(id);
        audioEvents.push({ event: "play", id, key });
      },
      stop: () => {
        activeAudio.delete(id);
        audioEvents.push({ event: "stop", id, key });
      },
      release: () => {
        if (released) return;
        released = true;
        presentationReleases.push(key);
        releasedPresentationIds.push(id);
        audioEvents.push({ event: "release", id, key });
      },
    };
  }
}

function available(request: VnMediaRequest<TestContext>, released: string[], byteLength = 4): VnMediaResolution {
  const media = request.slot.media;
  const bytes = new Uint8Array(byteLength).fill(1);
  let wasReleased = false;
  if (request.kind === "image") {
    return {
      status: "available", kind: "image", role: request.slot.role, media, revision: media.revision, variant: request.variant,
      descriptor: { kind: "image", mimeType: "image/png", byteLength: bytes.byteLength, width: 1, height: 1 },
      bytes,
      release: () => { if (!wasReleased) { wasReleased = true; released.push(media.key); } },
    };
  }
  return {
    status: "available", kind: "audio", role: request.slot.role, media, revision: media.revision, variant: request.variant,
    descriptor: { kind: "audio", mimeType: "audio/wav", byteLength: bytes.byteLength, durationMs: 100, sampleRate: 8000, channels: 1 },
    bytes,
    release: () => { if (!wasReleased) { wasReleased = true; released.push(media.key); } },
  };
}

function playerOptions(reader: Awaited<ReturnType<typeof openVn>>, harness: ReturnType<typeof mediaHarness>, context: VnPlayerContext<TestContext> = { media: { scope: "reader-one" }, content: { languages: ["en"] } }) {
  return {
    reader,
    text: textResolver(),
    resolveMedia: harness.resolveMedia,
    presentMedia: harness.presentMedia,
    context,
    selectVariant: (slot: { role: string }) => slot.role === "background" ? "desktop-cover" : "original",
    onLateDiscard: (request: VnMediaRequest<TestContext>) => { harness.lateDiscards.push(request.slot.media.key); },
  };
}

function mediaRequest(overrides: Partial<VnMediaRequest<TestContext>> = {}): VnMediaRequest<TestContext> {
  const media: MediaRef = { kind: "image", provider: "demo", key: "bg/room", revision: "r1" };
  return {
    requestId: "vn-media-1",
    execution,
    commit: "commit:" + "1".repeat(64),
    artifact: "artifact:" + "2".repeat(64),
    slot: { role: "background", id: "background", media },
    kind: "image",
    variant: "desktop-cover",
    context: { scope: "reader-one" },
    signal: new AbortController().signal,
    ...overrides,
  };
}

function candidateReader(base: VnReader, media: VnDependencyResult["media"], inspect: VnReader["inspect"]): VnReader {
  return {
    artifactId: base.artifactId, inspect,
    historyArtifacts: cursor => base.historyArtifacts(cursor),
    dependencies: (cursor, limits) => base.dependencies(cursor, limits),
    prepareCandidates: async (cursor, limits, signal) => ({
      status: signal.aborted ? "cancelled" : "complete",
      dependencies: { version: 1, artifact_id: base.artifactId, cursor, limits, complete: true,
        graphs: [], content: [], media, music_may_stop: false, truncations: [], unknowns: [],
        work: { locations: 1, scans: 0, graphs: 0, candidates: 1 } },
      objects: [],
    }),
    clearPreparedCandidates: () => base.clearPreparedCandidates(),
    advance: (cursor, dialogue) => base.advance(cursor, dialogue),
    choose: (cursor, point, options) => base.choose(cursor, point, options),
    checkout: cursor => base.checkout(cursor), exportSave: () => base.exportSave(),
    restore: save => base.restore(save), pack: () => base.pack(), reload: () => base.reload(),
    retrySave: () => base.retrySave(), close: () => base.close(),
  };
}

test.each(["invalid", "throw"])("a synchronous %s target selector releases an adopted warm lease", async mode => {
  const reader = await openVn({ pack: vnPack, execution });
  const harness = mediaHarness();
  let failSelector = false;
  const player = createVnPlayer({
    ...playerOptions(reader, harness), maxCandidateRequests: 2,
    selectVariant: slot => {
      if (failSelector && slot.role === "character") {
        if (mode === "throw") throw new Error("Host variant selection failed.");
        return "";
      }
      return slot.role === "background" ? "desktop-cover" : "original";
    },
    selectCandidateVariant: item => item.role.kind === "background" ? "desktop-cover" : "original",
    prepareCandidateMedia: async (resource, request) => harness.presentMedia(resource, request, await reader.inspect()),
  });
  try {
    await player.start();
    await player.advance(); await player.advance(); await player.advance();
    await vi.waitFor(() => expect(player.getSnapshot().candidates?.status).not.toBe("preparing"));
    const choice = await reader.inspect();
    if (choice.view.interaction.kind !== "choose") throw new Error("Expected a branch choice.");
    const route = choice.view.interaction.options.find(option => option.enabled);
    const warm = harness.presentations.find(item => item.key === "bg/hall");
    if (!route || !warm) throw new Error("Expected the warmed target background.");
    failSelector = true;
    await player.choose(route.id);
    expect(player.getSnapshot().status).toBe("unavailable");
    expect(harness.releasedPresentationIds.filter(id => id === warm.id)).toHaveLength(1);
    failSelector = false;
    await player.retryResources();
    expect(player.getSnapshot().status).toBe("ready");
    expect(harness.releasedPresentationIds.filter(id => id === warm.id)).toHaveLength(1);
  } finally { await player.close(); await reader.close(); }
});

test("automatic candidate work can be deferred until the host explicitly requests it", async () => {
  const reader = await openVn({ pack: vnPack, execution });
  const queries = vi.spyOn(reader, "prepareCandidates");
  const harness = mediaHarness();
  const player = createVnPlayer({ ...playerOptions(reader, harness), autoPrepareCandidates: false,
    selectCandidateVariant: item => item.role.kind === "background" ? "desktop-cover" : "original",
    prepareCandidateMedia: async (resource, request) => harness.presentMedia(resource, request, await reader.inspect()),
  });
  try {
    await player.start();
    await vi.waitFor(() => expect(player.getSnapshot().audioLoading).toBe(false));
    expect(queries).not.toHaveBeenCalled();
    await player.refreshPresentation();
    expect(queries).not.toHaveBeenCalled();
    await player.advance(); await player.advance(); await player.advance();
    expect(queries).not.toHaveBeenCalled();
    const required = harness.requests.length;
    await player.retryCandidates();
    expect(queries).toHaveBeenCalledTimes(1);
    expect(harness.requests.length).toBeGreaterThan(required);
    expect(player.getSnapshot().candidates?.mediaPrepared).toBeGreaterThan(0);
    await player.refreshPresentation();
    expect(queries).toHaveBeenCalledTimes(1);
    expect(player.getSnapshot().candidates).toBeUndefined();
  } finally { await player.close(); await reader.close(); }
});

test("changing authorization context removes the earlier candidate projection synchronously", async () => {
  const reader = await openVn({ pack: vnPack, execution });
  const harness = mediaHarness();
  const player = createVnPlayer({ ...playerOptions(reader, harness),
    selectCandidateVariant: item => item.role.kind === "background" ? "desktop-cover" : "original",
    prepareCandidateMedia: async (resource, request) => harness.presentMedia(resource, request, await reader.inspect()),
  });
  try {
    await player.start();
    await player.advance(); await player.advance(); await player.advance();
    await vi.waitFor(() => expect(player.getSnapshot().candidates?.projection).toBeDefined());
    const previous = player.getSnapshot().candidates?.projection;
    const switching = player.setContext({ media: { scope: "reader-two" }, content: { languages: ["en"], viewer: "reader-two" } });
    expect(player.getSnapshot().candidates?.projection).toBeUndefined();
    await switching;
    expect(player.getSnapshot().candidates?.projection).not.toBe(previous);
  } finally { await player.close(); await reader.close(); }
});

test("candidate admission counts audio that becomes pending after candidate work starts", async () => {
  const base = await openVn({ pack: vnPack, execution });
  const voice: MediaRef = { kind: "audio", provider: "demo", key: "voice/three", revision: "a1" };
  const background: MediaRef = { kind: "image", provider: "demo", key: "bg/candidate", revision: "r1" };
  const reader = candidateReader(base, [{ role: { kind: "background" }, reference: background }], async () => {
    const view = await base.inspect();
    return view.view.depth === 1 && view.view.interaction.kind === "advance"
      ? { ...view, view: { ...view.view, interaction: { ...view.view.interaction, voice } } } : view;
  });
  const harness = mediaHarness();
  let enabled = false;
  let started!: () => void;
  let release!: () => void;
  const candidateStarted = new Promise<void>(resolve => { started = resolve; });
  const candidateGate = new Promise<void>(resolve => { release = resolve; });
  const player = createVnPlayer({ ...playerOptions(reader, harness),
    limits: { ...limits(), maxEncodedBytesRetained: 16, maxDecodedBytesRetained: 64_000 },
    selectCandidateVariant: () => "candidate",
    prepareCandidateMedia: async (resource, request) => {
      if (!enabled) throw new DOMException("Candidate work is deferred.", "AbortError");
      started();
      await candidateGate;
      return harness.presentMedia(resource, request, await reader.inspect());
    },
  });
  try {
    await player.start();
    await vi.waitFor(() => expect(player.getSnapshot().audioLoading).toBe(false));
    await player.setSoundEnabled(true);
    harness.deferPresentationOnce("voice/three");
    harness.deferPlayOnce("voice/three");
    await player.advance();
    await vi.waitFor(() => expect(harness.hasPendingPresentation()).toBe(true));
    enabled = true;
    const warming = player.retryCandidates();
    await candidateStarted;
    harness.resolveDeferredPresentation();
    await vi.waitFor(() => expect(harness.hasPendingPlay()).toBe(true));
    release();
    await warming;
    expect(player.getSnapshot().status).toBe("ready");
    expect(player.getSnapshot().candidates?.status).toBe("partial");
    expect(player.getSnapshot().candidates?.mediaPrepared).toBe(0);
    expect(player.getSnapshot().candidates?.mediaUnavailable).toBe(1);
  } finally {
    release();
    if (harness.hasPendingPlay()) harness.resolveDeferredPlay();
    await player.close(); await base.close();
  }
});

test("checked media responses reject kind, exact revision, selected variant and descriptor mismatches", () => {
  const request = mediaRequest();
  const valid = available(request, []);
  const checked = checkedVnMediaResolution(request, valid, {
    maxRequests: 8, maxConcurrent: 3, maxEncodedBytesPerTarget: 1024, maxEncodedBytesRetained: 2048,
    maxDecodedBytesRetained: 2048, maxImageDimension: 64, maxAudioDurationMs: 1000,
  });
  expect(checked.status).toBe("available");
  if (checked.status !== "available") throw new Error("Expected a checked available image.");

  const wrongKind = { ...valid, kind: "audio", role: "background" } as unknown;
  expect(() => checkedVnMediaResolution(request, wrongKind, limits())).toThrowError(expect.objectContaining({ issue: "kind" }));
  const wrongRole = { ...valid, role: "voice" };
  expect(() => checkedVnMediaResolution(request, wrongRole, limits())).toThrowError(expect.objectContaining({ issue: "role" }));
  const wrongRevision = { ...valid, revision: "r2", media: { ...request.slot.media, revision: "r2" } };
  expect(() => checkedVnMediaResolution(request, wrongRevision, limits())).toThrowError(expect.objectContaining({ issue: "revision" }));
  const wrongVariant = { ...valid, variant: "mobile-cover" };
  expect(() => checkedVnMediaResolution(request, wrongVariant, limits())).toThrowError(expect.objectContaining({ issue: "variant" }));
  if (valid.status !== "available") throw new Error("Expected an available media fixture.");
  const invalidDescriptor = { ...valid, descriptor: { ...valid.descriptor, mimeType: "image/svg+xml" } } as unknown;
  expect(() => checkedVnMediaResolution(request, invalidDescriptor, limits())).toThrowError(expect.objectContaining({ issue: "descriptor" }));
  const unapprovedAddress = { ...valid, signedUrl: "host-private-address" } as unknown;
  expect(() => checkedVnMediaResolution(request, unapprovedAddress, limits())).toThrowError(expect.objectContaining({ issue: "descriptor" }));
});

test("player prepares direct candidate media with host-selected variants and reuses matching leases", async () => {
  const reader = await openVn({ pack: vnPack, execution });
  const harness = mediaHarness();
  let candidatePresentations = 0;
  let candidateId = 1000;
  const player = createVnPlayer({
    ...playerOptions(reader, harness),
    selectCandidateVariant: item => item.role.kind === "background" ? "desktop-cover" : "original",
    prepareCandidateMedia: async (resource, request) => {
      candidatePresentations++;
      const id = ++candidateId;
      if (resource.kind === "image") return { kind: "image", source: `candidate:${request.slot.media.key}:${id}`, release: () => undefined };
      return { kind: "audio", playFromStart: async () => undefined, stop: () => undefined, release: () => undefined };
    },
  });
  try {
    await player.start();
    await player.advance();
    await player.advance();
    await player.advance();
    await vi.waitFor(() => expect(player.getSnapshot().candidates?.status).not.toBe("preparing"));
    expect(player.getSnapshot().candidates?.mediaPrepared).toBeGreaterThan(0);
    expect(candidatePresentations).toBe(player.getSnapshot().candidates?.mediaPrepared);

    const before = await reader.inspect();
    const projection = await reader.dependencies(before.view.cursor, { locations: 512, graphs: 32, candidates: 512, references: 2048, scans: 8192 });
    const requestCountsBeforeTransition = new Map<string, number>();
    for (const request of harness.requests) requestCountsBeforeTransition.set(request.slot.media.key, (requestCountsBeforeTransition.get(request.slot.media.key) ?? 0) + 1);
    if (before.view.interaction.kind !== "choose") throw new Error("Expected the candidate branch choice.");
    const left = before.view.interaction.options.find(option => option.key === "left");
    if (!left) throw new Error("Expected the left candidate route.");
    await player.choose(left.id);
    const after = await reader.inspect();
    const nextMediaKeys = [after.view.scene.background?.key, ...after.view.scene.layers.map(layer => layer.asset.key), after.view.scene.music?.key]
      .filter((key): key is string => key !== undefined);
    const warmedKeys = projection.media.map(item => item.reference.key);
    expect(nextMediaKeys.some(key => warmedKeys.includes(key))).toBe(true);
    for (const key of nextMediaKeys.filter(key => warmedKeys.includes(key))) {
      expect(harness.requests.filter(request => request.slot.media.key === key).length,
        `${key}: before ${requestCountsBeforeTransition.get(key) ?? 0}, after ${harness.requests.filter(request => request.slot.media.key === key).map(request => `${request.slot.role}/${request.variant}`).join(",")}`)
        .toBe(requestCountsBeforeTransition.get(key) ?? 0);
    }
  } finally {
    await player.close();
    await reader.close();
  }
});

test("the public player prepares cold helper media before entry and reuses the matching displayed resource", async () => {
  const parts = splitPack(vnDependencyPack);
  const chunkRequests: string[] = [];
  const reader = await openVn({ manifest: parts.manifest, execution, fetchChunk: async id => {
    chunkRequests.push(id);
    const bytes = parts.objects.get(id);
    if (!bytes) throw new Error(`Missing fixture object ${id}`);
    return bytes;
  } });
  const harness = mediaHarness();
  const player = createVnPlayer({
    ...playerOptions(reader, harness),
    selectCandidateVariant: item => item.role.kind === "background" ? "desktop-cover" : "original",
    prepareCandidateMedia: async (resource, request) => harness.presentMedia(resource, request, await reader.inspect()),
  });
  try {
    await player.start();
    for (let step = 0; player.getSnapshot().screen?.view.view.interaction.kind === "advance" && step < 16; step++) await player.advance();
    await vi.waitFor(() => expect(player.getSnapshot().candidates?.status).not.toBe("preparing"));
    const candidate = player.getSnapshot().candidates;
    if (!candidate?.projection) throw new Error("Expected a checked candidate projection.");
    expect(candidate.projection.media.some(item => item.reference.key === "bg/helper")).toBe(true);
    expect(candidate.projection.media.some(item => item.reference.key === "music/helper")).toBe(true);
    expect(candidate.graphObjectsPrepared).toBeGreaterThan(0);
    const helperId = parts.chunks.map(objectId).find(id => chunkRequests.indexOf(id) > 0);
    if (!helperId) throw new Error("Expected the cold helper object to be checked.");
    expect(chunkRequests.filter(id => id === helperId)).toHaveLength(1);

    const choice = player.getSnapshot().screen?.view;
    if (!choice || choice.view.interaction.kind !== "choose") throw new Error("Expected the candidate choice.");
    const right = choice.view.interaction.options.filter(option => option.enabled)[1];
    if (!right) throw new Error("Expected the cross-graph route.");
    const bgRequestsBefore = harness.requests.filter(request => request.slot.media.key === "bg/hall").length;
    await player.choose(right.id);
    expect(player.getSnapshot().screen?.view.view.scene.background?.key).toBe("bg/hall");
    expect(harness.requests.filter(request => request.slot.media.key === "bg/hall")).toHaveLength(bgRequestsBefore);
    expect(chunkRequests.filter(id => id === helperId)).toHaveLength(1);
  } finally {
    await player.close();
    await reader.close();
  }
});

test("candidate cancellation releases earlier A before deferred B and releases late B once", async () => {
  const parts = splitPack(vnDependencyPack);
  const reader = await openVn({ manifest: parts.manifest, execution, fetchChunk: async id => {
    const bytes = parts.objects.get(id);
    if (!bytes) throw new Error(`Missing fixture object ${id}`);
    return bytes;
  } });
  const harness = mediaHarness();
  let currentView: VnViewV1 | undefined;
  const player = createVnPlayer({
    ...playerOptions(reader, harness),
    maxCandidateRequests: 2,
    selectCandidateVariant: item => item.role.kind === "background" ? "desktop-cover" : "original",
    prepareCandidateMedia: async (resource, request) => harness.presentMedia(resource, request, await reader.inspect()),
  });
  try {
    await player.start();
    harness.deferPresentationOnce("bg/helper");
    for (let step = 0; player.getSnapshot().screen?.view.view.interaction.kind === "advance" && step < 16; step++) await player.advance();
    currentView = player.getSnapshot().screen?.view;
    if (!currentView || currentView.view.interaction.kind !== "choose") throw new Error("Expected the fixture choice.");
    const start = harness.presentations.length;
    await vi.waitFor(() => expect(harness.hasPendingPresentation()).toBe(true));
    const candidates = harness.presentations.slice(start);
    const first = candidates.find(item => item.key !== "bg/helper");
    const deferred = candidates.find(item => item.key === "bg/helper");
    if (!first || !deferred) throw new Error("Expected a completed candidate A and deferred candidate B.");

    const left = currentView.view.interaction.options.find(option => option.enabled);
    if (!left) throw new Error("Expected an enabled route.");
    await player.choose(left.id);
    expect(harness.releasedPresentationIds.filter(id => id === first.id)).toHaveLength(1);
    expect(harness.releasedPresentationIds).not.toContain(deferred.id);

    harness.resolveDeferredPresentation();
    await vi.waitFor(() => expect(harness.releasedPresentationIds.filter(id => id === deferred.id)).toHaveLength(1));
  } finally {
    await player.close();
    await reader.close();
  }
});

test("a failed required target releases adopted warm assets and retries without a poisoned cache", async () => {
  const reader = await openVn({ pack: vnPack, execution });
  const harness = mediaHarness();
  const player = createVnPlayer({
    ...playerOptions(reader, harness),
    maxCandidateRequests: 2,
    selectCandidateVariant: item => item.role.kind === "background" ? "desktop-cover" : "original",
    prepareCandidateMedia: async (resource, request) => {
      const shown = await reader.inspect();
      return harness.presentMedia(resource, request, shown);
    },
  });
  try {
    await player.start();
    await player.advance();
    await player.advance();
    await player.advance();
    await vi.waitFor(() => expect(player.getSnapshot().candidates?.status).not.toBe("preparing"));
    expect(player.getSnapshot().candidates?.mediaPrepared).toBeGreaterThan(0);
    const choice = await reader.inspect();
    if (choice.view.interaction.kind !== "choose") throw new Error("Expected the fixture choice.");
    const left = choice.view.interaction.options.find(option => option.enabled);
    if (!left) throw new Error("Expected the ready left route.");
    const warmBackground = harness.presentations.find(item => item.key === "bg/hall");
    if (!warmBackground) throw new Error("Expected the target background to be warmed.");
    const before = harness.requests.filter(request => request.slot.media.key === "bg/hall").length;
    harness.failOnce("portrait/hero-alert");
    await player.choose(left.id);
    expect(player.getSnapshot().status).toBe("unavailable");
    expect(harness.releasedPresentationIds.filter(id => id === warmBackground.id)).toHaveLength(1);

    await player.retryResources();
    expect(player.getSnapshot().status).toBe("ready");
    expect(harness.requests.filter(request => request.slot.media.key === "bg/hall")).toHaveLength(before + 1);
    expect(harness.releasedPresentationIds.filter(id => id === warmBackground.id)).toHaveLength(1);
  } finally {
    await player.close();
    await reader.close();
  }
});

test("candidate media budgets include audio leases pending during current playback", async () => {
  const baseReader = await openVn({ pack: vnPack, execution });
  const candidateVoice: MediaRef = { kind: "audio", provider: "demo", key: "voice/three", revision: "a1" };
  const candidateBackground: MediaRef = { kind: "image", provider: "demo", key: "bg/candidate", revision: "r1" };
  let candidateEnabled = false;
  const reader: VnReader = {
    artifactId: baseReader.artifactId,
    inspect: async () => {
      const view = await baseReader.inspect();
      return view.view.depth === 1 && view.view.interaction.kind === "advance"
        ? { ...view, view: { ...view.view, interaction: { ...view.view.interaction, voice: candidateVoice } } }
        : view;
    },
    historyArtifacts: expected => baseReader.historyArtifacts(expected),
    dependencies: (cursor, limits) => baseReader.dependencies(cursor, limits),
    prepareCandidates: async (cursor, limits: VnDependencyLimits, signal): Promise<VnCandidatePreparation> => {
      const view = await baseReader.inspect();
      const dependencies: VnDependencyResult = {
        version: 1, artifact_id: view.view.artifact_id, cursor,
        limits, complete: true, graphs: [], content: [],
        media: [{ role: { kind: "background" }, reference: candidateBackground }],
        music_may_stop: false, truncations: [], unknowns: [],
        work: { locations: 1, scans: 0, graphs: 0, candidates: 1 },
      };
      return { status: signal.aborted ? "cancelled" : "complete", dependencies, objects: [] };
    },
    clearPreparedCandidates: () => baseReader.clearPreparedCandidates(),
    advance: (expected, dialogue) => baseReader.advance(expected, dialogue),
    choose: (expected, point, options) => baseReader.choose(expected, point, options),
    checkout: commit => baseReader.checkout(commit),
    exportSave: () => baseReader.exportSave(),
    restore: save => baseReader.restore(save),
    pack: () => baseReader.pack(),
    reload: () => baseReader.reload(),
    retrySave: () => baseReader.retrySave(),
    close: () => baseReader.close(),
  };
  const harness = mediaHarness();
  harness.deferPlayOnce("voice/three");
  const player = createVnPlayer({
    ...playerOptions(reader, harness),
    limits: { ...limits(), maxEncodedBytesRetained: 16, maxDecodedBytesRetained: 64_000 },
    selectCandidateVariant: () => "candidate",
    prepareCandidateMedia: async (resource, request) => {
      if (!candidateEnabled) throw new DOMException("Candidate preparation is deferred by the fixture.", "AbortError");
      return harness.presentMedia(resource, request, await reader.inspect());
    },
  });
  try {
    await player.start();
    await vi.waitFor(() => expect(player.getSnapshot().audioLoading).toBe(false));
    await player.setSoundEnabled(true);
    expect(player.getSnapshot().soundStatus).toBe("on");
    harness.deferPlayOnce("voice/three");
    await player.advance();
    const screenInteraction = player.getSnapshot().screen?.view.view.interaction;
    if (screenInteraction?.kind !== "advance") throw new Error("Expected the voice-bearing line.");
    expect(screenInteraction.voice?.key).toBe("voice/three");
    expect(harness.requests.some(request => request.slot.media.key === "voice/three")).toBe(true);
    await vi.waitFor(() => expect(harness.hasPendingPlay()).toBe(true));
    expect(player.getSnapshot().audioLoading).toBe(true);
    candidateEnabled = true;
    await player.retryCandidates();
    await vi.waitFor(() => expect(player.getSnapshot().candidates?.status).not.toBe("preparing"));
    expect(player.getSnapshot().status).toBe("ready");
    expect(player.getSnapshot().candidates?.status).toBe("partial");
    expect(player.getSnapshot().candidates?.mediaUnavailable).toBeGreaterThan(0);
    harness.resolveDeferredPlay();
  } finally {
    await player.close();
    await baseReader.close();
  }
});

test("a pre-commit missing graph chunk retries the original expected dialogue input", async () => {
  const parts = splitPack(lazyVnPack);
  let failChunk = false;
  const reader = await openVn({
    manifest: parts.manifest,
    execution,
    fetchChunk: async id => {
      const bytes = parts.objects.get(id);
      if (!bytes) throw new Error("Unknown test chunk.");
      if (failChunk) { failChunk = false; throw new Error("temporary graph outage"); }
      return bytes;
    },
  });
  const harness = mediaHarness();
  const advance = vi.spyOn(reader, "advance");
  const player = createVnPlayer(playerOptions(reader, harness));
  try {
    await player.start();
    const before = await reader.inspect();
    failChunk = true;
    await player.advance();
    expect(player.getSnapshot().status).toBe("logical-error");
    expect(player.getSnapshot().failure).toEqual({ kind: "logical", action: "advance", retryable: true });
    expect((await reader.inspect()).view.cursor).toBe(before.view.cursor);
    await player.retryInput();
    expect(player.getSnapshot().status).toBe("ready");
    expect(advance).toHaveBeenCalledTimes(2);
    expect((await reader.inspect()).view.depth).toBe(before.view.depth + 1);
  } finally {
    await player.close();
  }
});

test("resource retry presents the committed scene without a second Advance or Choose", async () => {
  const reader = await openVn({ pack: vnPack, execution });
  const harness = mediaHarness();
  const advance = vi.spyOn(reader, "advance");
  const choose = vi.spyOn(reader, "choose");
  const player = createVnPlayer(playerOptions(reader, harness));
  try {
    await player.start();
    await player.advance();
    await player.advance();
    await player.advance();
    const choiceView = await reader.inspect();
    if (choiceView.view.interaction.kind !== "choose") throw new Error("Expected a VN choice.");
    const option = choiceView.view.interaction.options.find(item => item.key === "left");
    if (!option) throw new Error("Expected the left branch.");
    harness.failOnce("bg/hall");
    await player.choose(option.id);
    expect(player.getSnapshot().status).toBe("unavailable");
    expect(player.getSnapshot().screen?.view.view.cursor).toBe(choiceView.view.cursor);
    const committed = await reader.inspect();
    expect(player.getSnapshot().target?.view.cursor).toBe(committed.view.cursor);
    const beforeRetry = { advance: advance.mock.calls.length, choose: choose.mock.calls.length, cursor: committed.view.cursor, depth: committed.view.depth };
    await player.retryResources();
    const after = await reader.inspect();
    expect(player.getSnapshot().status).toBe("ready");
    expect(after.view.cursor).toBe(beforeRetry.cursor);
    expect(after.view.depth).toBe(beforeRetry.depth);
    expect(advance).toHaveBeenCalledTimes(beforeRetry.advance);
    expect(choose).toHaveBeenCalledTimes(beforeRetry.choose);
    expect(harness.requests.filter(item => item.slot.media.key === "bg/hall")).toHaveLength(2);
  } finally {
    await player.close();
  }
});

test("unknown save outcome retries the pending bytes without a second Advance", async () => {
  const store = await IndexedDbStore.open(`vn-player-unknown-${crypto.randomUUID()}`);
  let loseAck = false;
  const writes: Uint8Array[] = [];
  const storage: CacheStore = {
    load: request => store.load(request),
    persist: async batch => {
      writes.push(batch.slice());
      const reply = await store.persist(batch);
      if (loseAck) { loseAck = false; throw new Error("persisted write acknowledgement lost"); }
      return reply;
    },
  };
  const reader = await openVn({ pack: vnPack, execution, storage });
  const harness = mediaHarness();
  const advance = vi.spyOn(reader, "advance");
  const player = createVnPlayer(playerOptions(reader, harness));
  try {
    await player.start();
    const before = await reader.inspect();
    loseAck = true;
    await player.advance();
    expect(player.getSnapshot().status).toBe("save-unknown");
    expect(player.getSnapshot().saveStatus).toBe("unknown");
    expect(advance).toHaveBeenCalledTimes(1);
    await player.retrySave();
    expect(player.getSnapshot().status).toBe("ready");
    expect(advance).toHaveBeenCalledTimes(1);
    expect((await reader.inspect()).view.depth).toBe(before.view.depth + 1);
    expect(writes.at(-1)).toEqual(writes.at(-2));
  } finally {
    await player.close();
    store.close();
  }
});

test("context change releases the old screen and discards its late media response", async () => {
  const reader = await openVn({ pack: vnPack, execution });
  const harness = mediaHarness();
  const player = createVnPlayer(playerOptions(reader, harness));
  try {
    await player.start();
    await player.advance();
    await player.advance();
    await player.advance();
    const choice = await reader.inspect();
    if (choice.view.interaction.kind !== "choose") throw new Error("Expected a VN choice.");
    const option = choice.view.interaction.options.find(item => item.key === "left");
    if (!option) throw new Error("Expected the left branch.");
    harness.delayOnce("bg/hall");
    const oldWork = player.choose(option.id);
    await vi.waitFor(() => expect(harness.requests.some(request => request.slot.media.key === "bg/hall" && request.context.scope === "reader-one")).toBe(true));
    await player.setContext({ media: { scope: "reader-two" }, content: { languages: ["en"], viewer: "reader-two" } });
    expect(player.getSnapshot().status).toBe("ready");
    expect(player.getSnapshot().screen?.view.view.scene.background?.key).toBe("bg/hall");
    harness.releaseLate();
    await oldWork;
    await vi.waitFor(() => expect(harness.lateDiscards).toContain("bg/hall"));
    expect(harness.hostReleases.filter(key => key === "bg/hall")).toHaveLength(2);
    expect(player.getSnapshot().status).toBe("ready");
    expect(player.getSnapshot().screen?.view.view.scene.background?.key).toBe("bg/hall");
  } finally {
    await player.close();
  }
});

test("resource limits report exhaustion instead of dropping a required image", async () => {
  const reader = await openVn({ pack: vnPack, execution });
  const harness = mediaHarness();
  const player = createVnPlayer({ ...playerOptions(reader, harness), limits: { maxRequests: 1 } });
  try {
    await player.start();
    expect(player.getSnapshot().status).toBe("unavailable");
    expect(player.getSnapshot().failure).toEqual({ kind: "resource", issue: "budget", retryable: true });
    expect(player.getSnapshot().screen).toBeUndefined();
    expect(harness.requests).toHaveLength(0);
  } finally {
    await player.close();
  }
});

test("a text failure cancels pending media, releases late leases, and allows a clean resource retry", async () => {
  const reader = await openVn({ pack: vnPack, execution });
  const harness = mediaHarness();
  harness.delayOnce("bg/room");
  let failText = true;
  const defaultText = textResolver();
  const text: VnTextResolver = {
    resolve: (request, signal) => failText ? Promise.reject(new VnTextUnavailableError()) : defaultText.resolve(request, signal),
  };
  const player = createVnPlayer({ ...playerOptions(reader, harness), text });
  try {
    await player.start();
    expect(player.getSnapshot().status).toBe("unavailable");
    expect(player.getSnapshot().failure).toEqual({ kind: "resource", issue: "text", retryable: true });
    await vi.waitFor(() => expect(harness.requests.some(request => request.slot.media.key === "bg/room")).toBe(true));
    harness.releaseLate();
    await vi.waitFor(() => expect(harness.lateDiscards).toContain("bg/room"));
    await vi.waitFor(() => expect(harness.hostReleases).toContain("bg/room"));

    failText = false;
    await player.retryResources();
    expect(player.getSnapshot().status).toBe("ready");
    expect(harness.requests.filter(request => request.slot.media.key === "bg/room")).toHaveLength(2);
  } finally {
    await player.close();
    await reader.close();
  }
  expect(new Set(harness.releasedPresentationIds).size).toBe(harness.releasedPresentationIds.length);
  expect([...harness.releasedPresentationIds].sort((left, right) => left - right))
    .toEqual(harness.presentations.map(item => item.id).sort((left, right) => left - right));
  expect(harness.hostReleases).toHaveLength(harness.requests.length);
});

test("context change stops and releases an unresolved old-context audio play", async () => {
  const reader = await openVn({ pack: vnPack, execution });
  const harness = mediaHarness();
  const player = createVnPlayer(playerOptions(reader, harness));
  try {
    await player.start();
    await vi.waitFor(() => expect(player.getSnapshot().screen?.assets.some(asset => asset.slot.role === "music")).toBe(true));
    const oldMusic = player.getSnapshot().screen?.assets.find(asset => asset.slot.role === "music");
    if (!oldMusic) throw new Error("Expected the initial music handle.");
    const oldPresentation = harness.presentations.find(item => item.kind === "audio" && item.key === oldMusic.media.key);
    if (!oldPresentation) throw new Error("Expected the initial audio presentation.");

    harness.deferPlayOnce(oldMusic.media.key);
    const enableSound = player.setSoundEnabled(true);
    await vi.waitFor(() => expect(harness.hasPendingPlay()).toBe(true));
    await player.setContext({ media: { scope: "reader-two" }, content: { languages: ["en"], viewer: "reader-two" } });
    expect(player.getSnapshot().status).toBe("ready");
    expect(player.getSnapshot().soundStatus).toBe("off");
    expect(harness.releasedPresentationIds.filter(id => id === oldPresentation.id)).toHaveLength(1);

    harness.resolveDeferredPlay();
    await enableSound.catch(() => undefined);
    expect(harness.activeAudio.has(oldPresentation.id)).toBe(false);
    expect(harness.audioEvents.filter(event => event.id === oldPresentation.id && event.event === "play")).toHaveLength(1);
    expect(harness.audioEvents.filter(event => event.id === oldPresentation.id && event.event === "stop").length).toBeGreaterThanOrEqual(2);
    expect(harness.releasedPresentationIds.filter(id => id === oldPresentation.id)).toHaveLength(1);
    expect(harness.requests.some(request => request.context.scope === "reader-two")).toBe(true);
  } finally {
    await player.close();
    await reader.close();
  }
});

test("close releases a late presenter result once and leaves the borrowed reader usable", async () => {
  const reader = await openVn({ pack: vnPack, execution });
  const harness = mediaHarness();
  harness.deferPresentationOnce("bg/room");
  const player = createVnPlayer(playerOptions(reader, harness));
  const starting = player.start();
  await vi.waitFor(() => expect(harness.hasPendingPresentation()).toBe(true));
  const pending = harness.presentations.find(item => item.kind === "image" && item.key === "bg/room");
  if (!pending) throw new Error("Expected the deferred background presentation.");

  await player.close();
  expect(player.getSnapshot().status).toBe("closed");
  harness.resolveDeferredPresentation();
  await starting;
  await vi.waitFor(() => expect(harness.releasedPresentationIds).toContain(pending.id));
  expect(harness.releasedPresentationIds.filter(id => id === pending.id)).toHaveLength(1);
  expect((await reader.inspect()).view.depth).toBe(0);
  await reader.close();
});

test.each(["target", "retained"] as const)("reused images count toward the %s budget", async budget => {
  const reader = await openVn({ pack: vnPack, execution });
  const initial = await reader.inspect();
  const initialVisualCount = (initial.view.scene.background ? 1 : 0) + initial.view.scene.layers.length;
  const hallBytes = budget === "target" ? initialVisualCount : 1;
  const harness = mediaHarness(request => request.kind === "image"
    ? request.slot.role === "background" && request.slot.media.key === "bg/hall" ? hallBytes : 1
    : 1);
  const player = createVnPlayer({
    ...playerOptions(reader, harness),
    limits: {
      maxEncodedBytesPerTarget: initialVisualCount,
      maxEncodedBytesRetained: budget === "retained" ? initialVisualCount : 64,
      maxDecodedBytesRetained: 1024,
    },
  });
  try {
    await player.start();
    await vi.waitFor(() => expect(player.getSnapshot().audioLoading).toBe(false));
    for (let index = 0; index < 3; index++) await player.advance();
    const choice = await reader.inspect();
    if (choice.view.interaction.kind !== "choose") throw new Error("Expected the VN choice.");
    const left = choice.view.interaction.options.find(option => option.key === "left");
    if (!left) throw new Error("Expected the left branch option.");
    const previousCharacters = new Set(choice.view.scene.layers.map(layer => layer.asset.key));
    const rawInspect = reader.inspect.bind(reader);
    vi.spyOn(reader, "inspect").mockImplementation(async () => {
      const view = await rawInspect();
      if (view.view.depth !== choice.view.depth + 1) return view;
      return { ...view, view: { ...view.view, scene: { ...view.view.scene, layers: choice.view.scene.layers } } };
    });
    const requestStart = harness.requests.length;

    await player.choose(left.id);
    expect(player.getSnapshot().status).toBe("unavailable");
    expect(player.getSnapshot().failure).toEqual({ kind: "resource", issue: "budget", retryable: true });
    expect(player.getSnapshot().screen?.view.view.cursor).toBe(choice.view.cursor);
    expect(player.getSnapshot().target?.view.depth).toBe(choice.view.depth + 1);
    const transitionRequests = harness.requests.slice(requestStart).map(request => request.slot.media.key);
    expect(transitionRequests).toEqual(["bg/hall"]);
    expect(transitionRequests.some(key => previousCharacters.has(key))).toBe(false);
  } finally {
    await player.close();
    await reader.close();
  }
});

test("an over-budget optional audio target leaves its visual scene ready and playable", async () => {
  const reader = await openVn({ pack: vnPack, execution });
  const initial = await reader.inspect();
  const visualCount = (initial.view.scene.background ? 1 : 0) + initial.view.scene.layers.length;
  const harness = mediaHarness(request => request.kind === "image" ? 1 : 2);
  const player = createVnPlayer({
    ...playerOptions(reader, harness),
    limits: { maxEncodedBytesPerTarget: visualCount, maxEncodedBytesRetained: 64, maxDecodedBytesRetained: 1024 },
  });
  try {
    await player.start();
    await vi.waitFor(() => expect(player.getSnapshot().audioLoading).toBe(false));
    expect(player.getSnapshot().status).toBe("ready");
    expect(player.getSnapshot().screen?.assets.every(asset => asset.presentation.kind === "image")).toBe(true);
    expect(player.getSnapshot().screen?.audioIssues).toContain("music");
    expect(harness.presentations.filter(item => item.kind === "audio").every(item => harness.releasedPresentationIds.includes(item.id))).toBe(true);
    const depth = player.getSnapshot().screen?.view.view.depth;
    await player.advance();
    expect(player.getSnapshot().status).toBe("ready");
    expect(player.getSnapshot().screen?.view.view.depth).toBe((depth ?? 0) + 1);
  } finally {
    await player.close();
    await reader.close();
  }
});

test("same-track Play, replacement Play, and Stop each issue one music stop command", async () => {
  const reader = await openVn({ pack: vnPack, execution });
  const initial = await reader.inspect();
  const sameTrack = initial.view.scene.music;
  if (!sameTrack) throw new Error("Expected the fixture music track.");
  const replacementTrack: MediaRef = { ...sameTrack, key: "music/replacement" };
  const overrides = new Map<number, { action: NonNullable<VnViewV1["view"]["music_action"]>; music: MediaRef | null }>([
    [1, { action: { kind: "play", track: sameTrack }, music: sameTrack }],
    [2, { action: { kind: "play", track: replacementTrack }, music: replacementTrack }],
    [3, { action: { kind: "stop" }, music: null }],
  ]);
  const rawInspect = reader.inspect.bind(reader);
  vi.spyOn(reader, "inspect").mockImplementation(async () => {
    const view = await rawInspect();
    const override = overrides.get(view.view.depth);
    return override ? {
      ...view,
      view: { ...view.view, music_action: override.action, scene: { ...view.view.scene, music: override.music } },
    } : view;
  });
  const harness = mediaHarness();
  const player = createVnPlayer(playerOptions(reader, harness));
  const musicEvents = () => harness.audioEvents.filter(event => event.key.startsWith("music/"));
  const eventCount = (event: "play" | "stop", key?: string) => musicEvents().filter(item => item.event === event && (!key || item.key === key)).length;
  try {
    await player.start();
    await vi.waitFor(() => expect(player.getSnapshot().screen?.assets.some(asset => asset.slot.role === "music")).toBe(true));
    await player.setSoundEnabled(true);

    let beforeStops = eventCount("stop", sameTrack.key);
    let beforePlays = eventCount("play", sameTrack.key);
    await player.advance();
    expect(player.getSnapshot().screen?.view.view.depth).toBe(1);
    expect(eventCount("stop", sameTrack.key) - beforeStops).toBe(1);
    expect(eventCount("play", sameTrack.key) - beforePlays).toBe(1);

    beforeStops = eventCount("stop", sameTrack.key);
    const replacementStops = eventCount("stop", replacementTrack.key);
    const replacementPlays = eventCount("play", replacementTrack.key);
    await player.advance();
    await vi.waitFor(() => expect(player.getSnapshot().audioLoading).toBe(false));
    expect(player.getSnapshot().screen?.view.view.depth).toBe(2);
    expect(eventCount("stop", sameTrack.key) - beforeStops).toBe(1);
    expect(eventCount("play", replacementTrack.key) - replacementPlays).toBe(1);

    beforeStops = eventCount("stop", replacementTrack.key);
    await player.advance();
    expect(player.getSnapshot().screen?.view.view.depth).toBe(3);
    expect(eventCount("stop", replacementTrack.key) - beforeStops).toBe(1);
  } finally {
    await player.close();
    await reader.close();
  }
});

test("closing with an unknown save keeps the reader's original pending-write retry path", async () => {
  const store = await IndexedDbStore.open(`vn-player-close-unknown-${crypto.randomUUID()}`);
  let failOnce = false;
  const writes: Uint8Array[] = [];
  const storage: CacheStore = {
    load: request => store.load(request),
    persist: async batch => {
      writes.push(batch.slice());
      const reply = await store.persist(batch);
      if (failOnce) { failOnce = false; throw new Error("persisted write acknowledgement lost"); }
      return reply;
    },
  };
  const reader = await openVn({ pack: vnPack, execution, storage });
  const advance = vi.spyOn(reader, "advance");
  const player = createVnPlayer(playerOptions(reader, mediaHarness()));
  try {
    await player.start();
    const before = await reader.inspect();
    failOnce = true;
    await player.advance();
    expect(player.getSnapshot().status).toBe("save-unknown");
    await player.close();
    expect(player.getSnapshot()).toMatchObject({ status: "closed", saveStatus: "unknown" });

    try {
      expect(await reader.retrySave()).toBe(true);
    } catch (error) {
      if (!(error instanceof StoreSuperseded)) throw error;
      await reader.reload();
    }
    expect((await reader.inspect()).view.depth).toBe(before.view.depth + 1);
    expect(advance).toHaveBeenCalledTimes(1);
    expect(writes.at(-1)).toEqual(writes.at(-2));
  } finally {
    await reader.close();
    store.close();
  }
});

test("variant refresh leaves a pending logical save and input untouched", async () => {
  const store = await IndexedDbStore.open(`vn-player-refresh-pending-${crypto.randomUUID()}`);
  const started = deferred<void>();
  const releaseWrite = deferred<void>();
  let holdNextPersist = false;
  const storage: CacheStore = {
    load: request => store.load(request),
    persist: async batch => {
      if (holdNextPersist) {
        holdNextPersist = false;
        started.resolve();
        await releaseWrite.promise;
      }
      return store.persist(batch);
    },
  };
  const reader = await openVn({ pack: vnPack, execution, storage });
  const advance = vi.spyOn(reader, "advance");
  const player = createVnPlayer(playerOptions(reader, mediaHarness()));
  try {
    await player.start();
    const before = await reader.inspect();
    holdNextPersist = true;
    const inFlight = player.advance();
    await started.promise;
    expect(player.getSnapshot()).toMatchObject({ status: "submitting", saveStatus: "saving" });
    await player.refreshPresentation();
    expect(player.getSnapshot()).toMatchObject({ status: "submitting", saveStatus: "saving" });
    expect(advance).toHaveBeenCalledTimes(1);
    releaseWrite.resolve();
    await inFlight;
    expect(player.getSnapshot().status).toBe("ready");
    expect((await reader.inspect()).view.depth).toBe(before.view.depth + 1);
    expect(advance).toHaveBeenCalledTimes(1);
  } finally {
    releaseWrite.resolve();
    await player.close();
    await reader.close();
    store.close();
  }
});

test("an earlier variant refresh cannot unlock a pending logical save", async () => {
  const store = await IndexedDbStore.open(`vn-player-refresh-before-save-${crypto.randomUUID()}`);
  const started = deferred<void>();
  const releaseWrite = deferred<void>();
  let holdNextPersist = false;
  const storage: CacheStore = {
    load: request => store.load(request),
    persist: async batch => {
      if (holdNextPersist) {
        holdNextPersist = false;
        started.resolve();
        await releaseWrite.promise;
      }
      return store.persist(batch);
    },
  };
  const reader = await openVn({ pack: vnPack, execution, storage });
  const advance = vi.spyOn(reader, "advance");
  const harness = mediaHarness();
  let variant = "desktop-cover";
  const player = createVnPlayer({ ...playerOptions(reader, harness), selectVariant: (slot: { role: string }) => slot.role === "background" ? variant : "original" });
  let inFlight: Promise<void> | undefined;
  try {
    await player.start();
    await vi.waitFor(() => expect(player.getSnapshot().audioLoading).toBe(false));
    const before = await reader.inspect();
    harness.delayOnce("bg/room");
    variant = "portrait-cover";
    const refresh = player.refreshPresentation();
    await vi.waitFor(() => expect(harness.requests.some(request => request.variant === variant)).toBe(true));
    holdNextPersist = true;
    inFlight = player.advance();
    await started.promise;
    expect(player.getSnapshot()).toMatchObject({ status: "submitting", saveStatus: "saving" });
    harness.releaseLate();
    await refresh;
    expect(player.getSnapshot()).toMatchObject({ status: "submitting", saveStatus: "saving" });
    await expect(player.advance()).rejects.toThrow("disabled");
    expect(advance).toHaveBeenCalledTimes(1);
    releaseWrite.resolve();
    await inFlight;
    expect((await reader.inspect()).view.depth).toBe(before.view.depth + 1);
  } finally {
    releaseWrite.resolve();
    await inFlight;
    await player.close();
    await reader.close();
    store.close();
  }
});

test("a superseded refresh cannot erase an unknown save or its retry", async () => {
  const store = await IndexedDbStore.open(`vn-player-refresh-unknown-${crypto.randomUUID()}`);
  let loseNextAck = false;
  const storage: CacheStore = {
    load: request => store.load(request),
    persist: async batch => {
      const reply = await store.persist(batch);
      if (loseNextAck) { loseNextAck = false; throw new Error("persisted acknowledgement lost"); }
      return reply;
    },
  };
  const reader = await openVn({ pack: vnPack, execution, storage });
  const advance = vi.spyOn(reader, "advance");
  const harness = mediaHarness();
  let variant = "desktop-cover";
  const player = createVnPlayer({ ...playerOptions(reader, harness), selectVariant: (slot: { role: string }) => slot.role === "background" ? variant : "original" });
  try {
    await player.start();
    await vi.waitFor(() => expect(player.getSnapshot().audioLoading).toBe(false));
    const before = await reader.inspect();
    harness.delayOnce("bg/room");
    variant = "portrait-cover";
    const refresh = player.refreshPresentation();
    await vi.waitFor(() => expect(harness.requests.some(request => request.variant === variant)).toBe(true));
    loseNextAck = true;
    await player.advance();
    expect(player.getSnapshot()).toMatchObject({ status: "save-unknown", saveStatus: "unknown" });
    harness.releaseLate();
    await refresh;
    expect(player.getSnapshot()).toMatchObject({ status: "save-unknown", saveStatus: "unknown" });
    await player.retrySave();
    expect(player.getSnapshot()).toMatchObject({ status: "ready", saveStatus: "saved" });
    expect((await reader.inspect()).view.depth).toBe(before.view.depth + 1);
    expect(advance).toHaveBeenCalledTimes(1);
  } finally {
    await player.close();
    await reader.close();
    store.close();
  }
});

test("a late sound toggle cannot unlock a pending save or interrupt Keep music", async () => {
  const store = await IndexedDbStore.open(`vn-player-audio-before-save-${crypto.randomUUID()}`);
  const started = deferred<void>();
  const releaseWrite = deferred<void>();
  let holdNextPersist = false;
  const storage: CacheStore = {
    load: request => store.load(request),
    persist: async batch => {
      if (holdNextPersist) { holdNextPersist = false; started.resolve(); await releaseWrite.promise; }
      return store.persist(batch);
    },
  };
  const reader = await openVn({ pack: vnPack, execution, storage });
  const harness = mediaHarness();
  const player = createVnPlayer(playerOptions(reader, harness));
  let inFlight: Promise<void> | undefined;
  try {
    await player.start();
    await vi.waitFor(() => expect(player.getSnapshot().audioLoading).toBe(false));
    harness.deferPlayOnce("music/theme");
    const sound = player.setSoundEnabled(true);
    await vi.waitFor(() => expect(harness.hasPendingPlay()).toBe(true));
    holdNextPersist = true;
    inFlight = player.advance();
    await started.promise;
    harness.resolveDeferredPlay();
    await sound;
    expect(player.getSnapshot()).toMatchObject({ status: "submitting", saveStatus: "saving" });
    const music = harness.presentations.find(item => item.key === "music/theme");
    expect(music).toBeDefined();
    expect(harness.activeAudio.has(music!.id)).toBe(true);
    releaseWrite.resolve();
    await inFlight;
    expect((await reader.inspect()).view.scene.music?.key).toBe("music/theme");
    expect(harness.activeAudio.has(music!.id)).toBe(true);
    expect(harness.audioEvents.filter(item => item.event === "stop" && item.key === "music/theme")).toHaveLength(0);
  } finally {
    releaseWrite.resolve();
    await inFlight;
    await player.close();
    await reader.close();
    store.close();
  }
});

test("turning sound off settles optional playback preparation and releases its candidate", async () => {
  const reader = await openVn({ pack: vnPack, execution });
  const harness = mediaHarness();
  harness.delayOnce("music/theme");
  harness.deferPlayOnce("music/theme");
  const player = createVnPlayer(playerOptions(reader, harness));
  try {
    await player.start();
    await player.setSoundEnabled(true);
    harness.releaseLate();
    await vi.waitFor(() => expect(harness.hasPendingPlay()).toBe(true));
    await player.setSoundEnabled(false);
    harness.resolveDeferredPlay();
    await vi.waitFor(() => expect(player.getSnapshot()).toMatchObject({ status: "ready", audioLoading: false, soundStatus: "off" }));
    expect(harness.activeAudio.size).toBe(0);
    expect(harness.presentationReleases.filter(key => key === "music/theme")).toHaveLength(1);
  } finally {
    await player.close();
    await reader.close();
  }
  expect(harness.releasedPresentationIds).toHaveLength(harness.presentations.length);
  expect(new Set(harness.releasedPresentationIds).size).toBe(harness.releasedPresentationIds.length);
});

test("turning sound off settles an audio retry awaiting playback", async () => {
  const reader = await openVn({ pack: vnPack, execution });
  const harness = mediaHarness();
  harness.failOnce("music/theme");
  const player = createVnPlayer(playerOptions(reader, harness));
  try {
    await player.start();
    await vi.waitFor(() => expect(player.getSnapshot().audioLoading).toBe(false));
    await player.setSoundEnabled(true);
    expect(player.getSnapshot().soundStatus).toBe("on");
    harness.deferPlayOnce("music/theme");
    const retry = player.retryAudio();
    await vi.waitFor(() => expect(harness.hasPendingPlay()).toBe(true));
    await player.setSoundEnabled(false);
    harness.resolveDeferredPlay();
    await retry;
    expect(player.getSnapshot()).toMatchObject({ status: "ready", audioLoading: false, soundStatus: "off" });
    expect(harness.activeAudio.size).toBe(0);
  } finally {
    await player.close();
    await reader.close();
  }
  expect(harness.releasedPresentationIds).toHaveLength(harness.presentations.length);
  expect(new Set(harness.releasedPresentationIds).size).toBe(harness.releasedPresentationIds.length);
});

test("an unresolved optional audio request leaves a muted ready scene interactive", async () => {
  const reader = await openVn({ pack: vnPack, execution });
  const harness = mediaHarness();
  harness.delayOnce("music/theme");
  const player = createVnPlayer(playerOptions(reader, harness));
  try {
    await player.start();
    await vi.waitFor(() => expect(harness.requests.some(request => request.slot.role === "music")).toBe(true));
    expect(player.getSnapshot()).toMatchObject({ status: "ready", soundStatus: "off", audioLoading: true });
    expect(player.getSnapshot().screen).toBeDefined();
    const before = await reader.inspect();
    await player.advance();
    expect(player.getSnapshot().status).toBe("ready");
    expect((await reader.inspect()).view.depth).toBe(before.view.depth + 1);
    harness.releaseLate();
    await vi.waitFor(() => expect(harness.lateDiscards).toContain("music/theme"));
    expect(player.getSnapshot().status).toBe("ready");
  } finally {
    await player.close();
    await reader.close();
  }
});

function deferred<T>() {
  let resolve!: (value: T | PromiseLike<T>) => void;
  let reject!: (error: unknown) => void;
  const promise = new Promise<T>((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, resolve, reject };
}

function limits() {
  return {
    maxRequests: 8,
    maxConcurrent: 3,
    maxEncodedBytesPerTarget: 1024,
    maxEncodedBytesRetained: 2048,
    maxDecodedBytesRetained: 2048,
    maxImageDimension: 64,
    maxAudioDurationMs: 1000,
  };
}
