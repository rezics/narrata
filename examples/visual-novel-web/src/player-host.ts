import { IndexedDbStore } from "@rezics/narrata/storage";
import type { CacheStore } from "@rezics/narrata/storage";
import { openVn, type MediaKind, type MediaRef, type VnReader, type VnViewV1 } from "@rezics/narrata/vn";
import {
  createVnPlayer,
  type VnMediaPresenter,
  type VnMediaResolution,
  type VnMediaRequest,
  type VnMediaResolver,
  type VnPlayerContext,
  type VnPlayerController,
  type VnPlayerOptions,
  type VnPresentedMedia,
} from "@rezics/narrata/vn/player";
import type { ResolveItem, ResolveRequest, Resolution } from "@rezics/narrata/reader";
import { vnDemoArtifactId, vnDemoManifestPath, vnDemoObjectDirectory } from "./generated/publication.js";

export interface DemoContext {
  readonly scope: string;
}

export interface DemoMetrics {
  readonly logicalRequests: Array<{ readonly kind: "manifest" | "chunk"; readonly id: string; readonly bytes: number }>;
  readonly contentRequests: Array<{ provider: string; key: string; revision?: string; bytes: number; status: string }>;
  readonly mediaRequests: Array<{ readonly role: string; readonly kind: string; readonly provider: string; readonly key: string; readonly revision: string; readonly variant: string; readonly contextScope: string; bytes: number; status: string }>;
  readonly lateDiscarded: Array<{ readonly role: string; readonly key: string; readonly revision: string; readonly variant: string; readonly artifact: string; readonly commit: string; readonly contextScope: string }>;
  readonly audioEvents: Array<{ readonly event: string; readonly role: string; readonly key: string; readonly revision: string; readonly commit: string; readonly offsetSeconds?: number; readonly loop?: boolean }>;
  readonly saveWrites: Array<{ readonly bytes: number; readonly sha256: string }>;
  readonly readerCalls: { advance: number; choose: number; checkout: number; inspect: number };
  aborts: number;
  lateDiscards: number;
  releases: number;
  retries: { resource: number; audio: number; logical: number; save: number };
}

export interface DemoControls {
  failNext?: { key: string; delayMs: number };
  delayLateNext?: { key: string; delayMs: number };
  corruptAudioKey?: string;
  failNextSave?: boolean;
}

export interface DemoHost {
  readonly controller: VnPlayerController<DemoContext>;
  readonly reader: VnReader;
  readonly metrics: DemoMetrics;
  readonly controls: DemoControls;
  readonly isClosed: boolean;
  switchContext(scope: string): Promise<void>;
  unlockAudio(): Promise<void>;
  close(): Promise<void>;
}

export interface VnCandidateSnapshot {
  readonly artifactId: string;
  readonly manifest: Uint8Array;
  readonly objects: ReadonlyMap<string, Uint8Array>;
}

export interface DemoHostOptions {
  readonly candidate?: VnCandidateSnapshot;
  readonly contentDrafts?: ReadonlyMap<string, { readonly text: string; readonly revision: string }>;
}

interface ImageAssetDefinition {
  readonly kind: "image";
  readonly path: string;
  readonly mimeType: "image/png";
  readonly width: number;
  readonly height: number;
}

interface AudioAssetDefinition {
  readonly kind: "audio";
  readonly path: string;
  readonly mimeType: "audio/wav";
}

type AssetDefinition = ImageAssetDefinition | AudioAssetDefinition;

const EXECUTION = "execution:0190f2a0000070008000000000000001";
const IMAGE_ASSETS: ReadonlyMap<string, ImageAssetDefinition> = new Map([
  [mediaKey("image", "demo", "background/river-night", "1"), { kind: "image", path: "/assets/river-night-wide.png", mimeType: "image/png", width: 1586, height: 992 }],
  [mediaKey("image", "demo", "background/old-hall", "1"), { kind: "image", path: "/assets/old-hall-interior.png", mimeType: "image/png", width: 1586, height: 992 }],
  [mediaKey("image", "demo", "character/mira", "1"), { kind: "image", path: "/assets/mira-clean.png", mimeType: "image/png", width: 1024, height: 1536 }],
  [mediaKey("image", "demo", "character/tomas", "1"), { kind: "image", path: "/assets/tomas-clean.png", mimeType: "image/png", width: 1024, height: 1536 }],
]);
const AUDIO_ASSETS: ReadonlyMap<string, AudioAssetDefinition> = new Map([
  [mediaKey("audio", "demo", "music/river-road", "1"), { kind: "audio", path: "/assets/audio/river-road-theme.wav", mimeType: "audio/wav" }],
  [mediaKey("audio", "demo", "voice/mira-opening", "1"), { kind: "audio", path: "/assets/audio/mira-opening.wav", mimeType: "audio/wav" }],
  [mediaKey("audio", "demo", "voice/tomas-road", "1"), { kind: "audio", path: "/assets/audio/tomas-road.wav", mimeType: "audio/wav" }],
]);
const CONTENT_ASSETS = new Map([
  ["speaker.mira", { path: "/content/en/host/speaker.mira.txt", revision: "1" }],
  ["speaker.tomas", { path: "/content/en/host/speaker.tomas.txt", revision: "1" }],
  ["dialogue.mira.opening", { path: "/content/en/host/dialogue.mira.opening.txt", revision: "1" }],
  ["dialogue.tomas.road", { path: "/content/en/host/dialogue.tomas.road.txt", revision: "1" }],
  ["dialogue.mira.hall", { path: "/content/en/host/dialogue.mira.hall.txt", revision: "1" }],
  ["choice.prompt", { path: "/content/en/host/choice.prompt.txt", revision: "1" }],
  ["option.narrow", { path: "/content/en/host/option.narrow.txt", revision: "1" }],
  ["option.courtyard", { path: "/content/en/host/option.courtyard.txt", revision: "1" }],
  ["dialogue.narrow", { path: "/content/en/host/dialogue.narrow.txt", revision: "1" }],
  ["dialogue.courtyard", { path: "/content/en/host/dialogue.courtyard.txt", revision: "1" }],
  ["dialogue.converge", { path: "/content/en/host/dialogue.converge.txt", revision: "1" }],
]);

export async function openDemoHost(options: DemoHostOptions = {}): Promise<DemoHost> {
  const candidate = options.candidate;
  const metrics: DemoMetrics = {
    logicalRequests: [],
    contentRequests: [],
    mediaRequests: [],
    lateDiscarded: [],
    audioEvents: [],
    saveWrites: [],
    readerCalls: { advance: 0, choose: 0, checkout: 0, inspect: 0 },
    aborts: 0,
    lateDiscards: 0,
    releases: 0,
    retries: { resource: 0, audio: 0, logical: 0, save: 0 },
  };
  const controls: DemoControls = {};
  const manifest = candidate?.manifest ?? await fetchBytes(vnDemoManifestPath);
  metrics.logicalRequests.push({ kind: "manifest", id: candidate?.artifactId ?? vnDemoArtifactId, bytes: manifest.byteLength });

  const contentResolver = {
    resolve: (request: ResolveRequest, signal: AbortSignal) => resolveContent(request, signal, metrics, options.contentDrafts),
  };
  const mediaResolver: VnMediaResolver<DemoContext> = request => resolveMedia(request, metrics, controls);
  let audioContext: AudioContext;
  const presentMedia: VnMediaPresenter = async (resource, request, view) => present(resource, request, view, metrics, audioContext);
  const context: VnPlayerContext<DemoContext> = { media: { scope: "demo-public" }, content: { languages: ["en"], realization: "river-road:1", viewer: "demo-reader" } };
  const store = candidate ? undefined : await IndexedDbStore.open("narrata-visual-novel-demo");
  const storage: CacheStore | undefined = store ? {
    load: request => store.load(request),
    persist: async batch => {
      metrics.saveWrites.push({ bytes: batch.byteLength, sha256: await sha256(batch) });
      if (controls.failNextSave) {
        delete controls.failNextSave;
        await store.persist(batch);
        throw new Error("persisted write acknowledgement lost");
      }
      return store.persist(batch);
    },
  } : undefined;
  const reader = await openVn({
    manifest: candidate?.manifest ?? manifest,
    ...(storage ? { storage } : {}),
    execution: candidate ? `execution:${crypto.randomUUID().replaceAll("-", "")}` : EXECUTION,
    fetchChunk: async id => {
      if (!/^object:[0-9a-f]{64}$/.test(id)) throw new Error("The requested VN chunk identity is invalid.");
      const bytes = candidate?.objects.get(id) ?? (candidate ? undefined : await fetchBytes(`${vnDemoObjectDirectory}/${id.slice("object:".length)}.cbor`));
      if (!bytes) throw new Error("The checked VN preview omitted a referenced program object.");
      metrics.logicalRequests.push({ kind: "chunk", id, bytes: bytes.byteLength });
      return bytes.slice();
    },
  }).catch(error => {
    store?.close();
    throw error;
  });
  if (reader.artifactId !== (candidate?.artifactId ?? vnDemoArtifactId)) {
    try { await reader.close(); }
    finally { store?.close(); }
    throw new Error("The loaded VN manifest does not match its checked host candidate.");
  }
  const countedReader = countReader(reader, metrics);
  try { audioContext = new AudioContext({ latencyHint: "interactive" }); }
  catch (error) {
    try { await reader.close(); }
    finally { store?.close(); }
    throw error;
  }
  const playerOptions: VnPlayerOptions<DemoContext> = {
    reader: countedReader,
    text: contentResolver,
    resolveMedia: mediaResolver,
    presentMedia,
    context,
    selectVariant: slot => {
      if (slot.role === "background") return window.matchMedia("(max-width: 640px)").matches ? "portrait-cover" : "landscape-cover";
      return slot.role === "character" ? "full" : "original";
    },
    onLateDiscard: request => {
      metrics.lateDiscards++;
      metrics.lateDiscarded.push({ role: request.slot.role, key: request.slot.media.key, revision: request.slot.media.revision, variant: request.variant, artifact: request.artifact, commit: request.commit, contextScope: request.context.scope });
    },
  };
  const controller = createVnPlayer(playerOptions);
  let closed = false;
  let closing: Promise<void> | undefined;
  return {
    controller,
    reader: countedReader,
    metrics,
    controls,
    get isClosed() { return closed; },
    async switchContext(scope) {
      await controller.setContext({
        media: { scope },
        content: { languages: ["en"], realization: "river-road:1", viewer: scope },
      });
    },
    async unlockAudio() {
      await audioContext.resume();
    },
    close() {
      if (closing) return closing;
      closed = true;
      const controllerClose = controller.close();
      closing = (async () => {
        try {
          try { await controllerClose; }
          finally { await countedReader.close(); }
        } finally {
          store?.close();
          await audioContext.close();
        }
      })();
      return closing;
    },
  };
}

async function resolveContent(request: ResolveRequest, signal: AbortSignal, metrics: DemoMetrics, drafts?: DemoHostOptions["contentDrafts"]): Promise<readonly Resolution[]> {
  if (!request.context.languages?.includes("en")) return request.items.map(() => ({ status: "unavailable" }));
  return Promise.all(request.items.map(async (item: ResolveItem): Promise<Resolution> => {
    if (signal.aborted) throw abortError();
    const reference = "unit" in item.content ? item.content.unit : item.content;
    const row: DemoMetrics["contentRequests"][number] = { provider: reference.provider, key: reference.key, bytes: 0, status: "pending" };
    metrics.contentRequests.push(row);
    try {
      const draft = drafts?.get(reference.key);
      if (reference.provider === "host" && draft) {
        const bytes = new TextEncoder().encode(draft.text);
        row.bytes = bytes.byteLength;
        row.revision = draft.revision;
        row.status = "ok";
        return { status: "ok", revision: draft.revision, payload: { text: draft.text } };
      }
      const content = CONTENT_ASSETS.get(reference.key);
      if (reference.provider !== "host" || !content) {
        row.status = "unavailable";
        return { status: "unavailable" };
      }
      const responseBytes = await fetchBytes(content.path, signal);
      if (signal.aborted) throw abortError();
      row.bytes = responseBytes.byteLength;
      const text = new TextDecoder("utf-8", { fatal: true }).decode(responseBytes);
      row.revision = content.revision;
      row.status = "ok";
      return { status: "ok", revision: content.revision, payload: { text } };
    } catch (error) {
      if (signal.aborted) throw error;
      row.status = "unavailable";
      return { status: "unavailable" };
    }
  }));
}

async function resolveMedia(request: VnMediaRequest<DemoContext>, metrics: DemoMetrics, controls: DemoControls): Promise<VnMediaResolution> {
  const definition = resolveDefinition(request.slot.media, request.kind, request.variant);
  let done = false;
  const onAbort = () => { if (!done) metrics.aborts++; };
  request.signal.addEventListener("abort", onAbort, { once: true });
  const log: DemoMetrics["mediaRequests"][number] = {
    role: request.slot.role,
    kind: request.kind,
    provider: request.slot.media.provider,
    key: request.slot.media.key,
    revision: request.slot.media.revision,
    variant: request.variant,
    contextScope: request.context.scope,
    bytes: 0,
    status: "pending",
  };
  metrics.mediaRequests.push(log);
  try {
    const failed = controls.failNext?.key === request.slot.media.key ? controls.failNext : undefined;
    if (failed) {
      delete controls.failNext;
      await delay(failed.delayMs);
      log.status = "unavailable";
      return unavailable(request);
    }
    const late = controls.delayLateNext?.key === request.slot.media.key ? controls.delayLateNext : undefined;
    if (late) {
      delete controls.delayLateNext;
      await delay(late.delayMs);
      const bytes = await fetchBytes(definition.path);
      log.bytes = bytes.byteLength;
      log.status = "available";
      return available(request, definition, bytes, metrics, controls);
    }
    const bytes = await fetchBytes(definition.path, request.signal);
    if (request.signal.aborted) throw abortError();
    log.bytes = bytes.byteLength;
    log.status = "available";
    return available(request, definition, bytes, metrics, controls);
  } catch (error) {
    if (request.signal.aborted) throw error;
    log.status = "unavailable";
    return unavailable(request);
  } finally {
    done = true;
    request.signal.removeEventListener("abort", onAbort);
  }
}

function available(
  request: VnMediaRequest<DemoContext>,
  definition: AssetDefinition,
  bytes: Uint8Array,
  metrics: DemoMetrics,
  controls: DemoControls,
): VnMediaResolution {
  let released = false;
  const release = () => {
    if (released) return;
    released = true;
    metrics.releases++;
  };
  if (definition.kind === "image" && request.kind === "image") {
    return {
      status: "available",
      kind: "image",
      role: request.slot.role,
      media: request.slot.media,
      revision: request.slot.media.revision,
      variant: request.variant,
      descriptor: { kind: "image", mimeType: definition.mimeType, byteLength: bytes.byteLength, width: definition.width, height: definition.height },
      bytes,
      release,
    };
  }
  if (definition.kind !== "audio" || request.kind !== "audio") throw new Error("The host media registry kind is inconsistent.");
  const metadata = waveMetadata(bytes);
  const audioBytes = controls.corruptAudioKey === request.slot.media.key ? new Uint8Array([0, 1, 2, 3, 4, 5, 6, 7]) : bytes;
  if (controls.corruptAudioKey === request.slot.media.key) delete controls.corruptAudioKey;
  return {
    status: "available",
    kind: "audio",
    role: request.slot.role,
    media: request.slot.media,
    revision: request.slot.media.revision,
    variant: request.variant,
    descriptor: { kind: "audio", mimeType: definition.mimeType, byteLength: audioBytes.byteLength, durationMs: metadata.durationMs, sampleRate: metadata.sampleRate, channels: metadata.channels },
    bytes: audioBytes,
    release,
  };
}

function unavailable(request: VnMediaRequest<DemoContext>): VnMediaResolution {
  return {
    status: "unavailable",
    kind: request.kind,
    role: request.slot.role,
    media: request.slot.media,
    revision: request.slot.media.revision,
    variant: request.variant,
    reason: "temporary",
  };
}

async function present(
  resource: Parameters<NonNullable<VnPlayerOptions<DemoContext>["presentMedia"]>>[0],
  request: Parameters<NonNullable<VnPlayerOptions<DemoContext>["presentMedia"]>>[1],
  view: VnViewV1,
  metrics: DemoMetrics,
  audio: AudioContext,
): Promise<VnPresentedMedia> {
  const { slot, signal } = request;
  if (resource.kind === "image") {
    const source = URL.createObjectURL(new Blob([resource.bytes.slice().buffer], { type: resource.descriptor.mimeType }));
    const image = new Image();
    image.decoding = "async";
    image.src = source;
    try {
      await raceAbort(image.decode(), signal);
    } catch (error) {
      URL.revokeObjectURL(source);
      throw error;
    }
    let released = false;
    return {
      kind: "image",
      source,
      release: () => {
        if (released) return;
        released = true;
        URL.revokeObjectURL(source);
        metrics.releases++;
      },
    };
  }

  let buffer: AudioBuffer | undefined;
  try {
    const copy = resource.bytes.slice().buffer;
    buffer = await raceAbort(audio.decodeAudioData(copy), signal);
  } catch (error) {
    metrics.audioEvents.push({ event: "decode-error", role: slot.role, key: slot.media.key, revision: slot.media.revision, commit: view.view.cursor });
    throw error;
  }
  let source: AudioBufferSourceNode | undefined;
  let released = false;
  return {
    kind: "audio",
    async playFromStart(signal) {
      if (released || !buffer) throw new Error("The audio resource has been released.");
      await raceAbort(audio.resume(), signal);
      if (signal.aborted) throw abortError();
      if (released || !buffer) throw new Error("The audio resource has been released.");
      stopSource();
      const node = audio.createBufferSource();
      node.buffer = buffer;
      node.loop = slot.role === "music";
      node.connect(audio.destination);
      source = node;
      node.start(0, 0);
      metrics.audioEvents.push({ event: "play", role: slot.role, key: slot.media.key, revision: slot.media.revision, commit: view.view.cursor, offsetSeconds: 0, loop: node.loop });
    },
    stop: stopSource,
    release: () => {
      if (released) return;
      released = true;
      stopSource();
      buffer = undefined;
      metrics.releases++;
    },
  };

  function stopSource() {
    const previous = source;
    source = undefined;
    if (!previous) return;
    try { previous.stop(); } catch { /* A short voice can end before the next line starts. */ }
    previous.disconnect();
    metrics.audioEvents.push({ event: "stop", role: slot.role, key: slot.media.key, revision: slot.media.revision, commit: view.view.cursor });
  }
}

function resolveDefinition(media: Readonly<MediaRef>, kind: MediaKind, variant: string): AssetDefinition {
  const definition = (kind === "image" ? IMAGE_ASSETS : AUDIO_ASSETS).get(mediaKey(media.kind, media.provider, media.key, media.revision));
  if (!definition || definition.kind !== kind) throw new Error("The exact VN media version is not in the demo host catalog.");
  if (definition.kind === "image" && media.key === "background/river-night" && variant === "portrait-cover") {
    return { ...definition, path: "/assets/river-night-portrait.png", width: 853, height: 1844 };
  }
  return definition;
}

function mediaKey(kind: MediaKind, provider: string, key: string, revision: string): string {
  return JSON.stringify([kind, provider, key, revision]);
}

function countReader(reader: VnReader, metrics: DemoMetrics): VnReader {
  return {
    get artifactId() { return reader.artifactId; },
    inspect: async () => { metrics.readerCalls.inspect++; return reader.inspect(); },
    historyArtifacts: expected => reader.historyArtifacts(expected),
    advance: (expected, dialogue) => { metrics.readerCalls.advance++; return reader.advance(expected, dialogue); },
    choose: (expected, point, options) => { metrics.readerCalls.choose++; return reader.choose(expected, point, options); },
    checkout: commit => { metrics.readerCalls.checkout++; return reader.checkout(commit); },
    exportSave: () => reader.exportSave(),
    restore: save => reader.restore(save),
    pack: () => reader.pack(),
    reload: () => reader.reload(),
    retrySave: () => reader.retrySave(),
    close: () => reader.close(),
  };
}

async function fetchBytes(path: string, signal?: AbortSignal): Promise<Uint8Array> {
  const response = await fetch(path, signal ? { signal, cache: "no-cache" } : { cache: "no-cache" });
  if (!response.ok) throw new Error("The requested host resource is unavailable.");
  const bytes = new Uint8Array(await response.arrayBuffer());
  const declared = Number(response.headers.get("content-length"));
  if (Number.isSafeInteger(declared) && declared >= 0 && declared !== bytes.byteLength) throw new Error("A host resource had an inconsistent byte length.");
  return bytes;
}

async function sha256(bytes: Uint8Array): Promise<string> {
  const digest = await crypto.subtle.digest("SHA-256", bytes.slice().buffer);
  return [...new Uint8Array(digest)].map(byte => byte.toString(16).padStart(2, "0")).join("");
}

function waveMetadata(bytes: Uint8Array): { sampleRate: number; channels: number; durationMs: number } {
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  if (bytes.byteLength < 44 || readAscii(bytes, 0, 4) !== "RIFF" || readAscii(bytes, 8, 4) !== "WAVE") throw new Error("The WAV asset is invalid.");
  let sampleRate = 0;
  let channels = 0;
  let bitsPerSample = 0;
  let dataBytes = 0;
  for (let offset = 12; offset + 8 <= bytes.byteLength;) {
    const name = readAscii(bytes, offset, 4);
    const length = view.getUint32(offset + 4, true);
    const start = offset + 8;
    if (start + length > bytes.byteLength) throw new Error("The WAV asset is truncated.");
    if (name === "fmt ") {
      channels = view.getUint16(start + 2, true);
      sampleRate = view.getUint32(start + 4, true);
      bitsPerSample = view.getUint16(start + 14, true);
    } else if (name === "data") dataBytes = length;
    offset = start + length + (length % 2);
  }
  if (sampleRate < 8000 || channels < 1 || bitsPerSample < 8 || dataBytes < 1) throw new Error("The WAV asset metadata is invalid.");
  return { sampleRate, channels, durationMs: Math.round(dataBytes * 1000 / (sampleRate * channels * (bitsPerSample / 8))) };
}

function readAscii(bytes: Uint8Array, offset: number, length: number): string {
  return String.fromCharCode(...bytes.subarray(offset, offset + length));
}

function delay(milliseconds: number): Promise<void> {
  return new Promise(resolve => setTimeout(resolve, milliseconds));
}

function raceAbort<T>(promise: Promise<T>, signal: AbortSignal): Promise<T> {
  if (signal.aborted) return Promise.reject(abortError());
  return new Promise<T>((resolve, reject) => {
    const cleanup = () => signal.removeEventListener("abort", abort);
    const abort = () => { cleanup(); reject(abortError()); };
    signal.addEventListener("abort", abort, { once: true });
    promise.then(
      value => { cleanup(); resolve(value); },
      error => { cleanup(); reject(error); },
    );
  });
}

function abortError(): DOMException {
  return new DOMException("The host request was cancelled.", "AbortError");
}
