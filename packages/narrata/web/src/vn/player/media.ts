import type { ArtifactId, CommitId, ExecutionId, MediaKind, MediaRef, VnViewV1 } from "../../generated/vn-view.js";

export type VnMediaRole = "background" | "character" | "music" | "voice";

export type VnMediaSlot =
  | { role: "background"; id: "background"; media: MediaRef }
  | { role: "character"; id: string; order: number; media: MediaRef }
  | { role: "music"; id: "music"; media: MediaRef }
  | { role: "voice"; id: "voice"; media: MediaRef };

export interface VnMediaRequest<HostContext> {
  readonly requestId: string;
  readonly execution: ExecutionId;
  readonly commit: CommitId;
  readonly artifact: ArtifactId;
  readonly slot: VnMediaSlot;
  readonly kind: MediaKind;
  readonly variant: string;
  /** Captured host authorization/content context. It is never cloned or persisted. */
  readonly context: HostContext;
  readonly signal: AbortSignal;
}

export interface VnImageDescriptor {
  readonly kind: "image";
  readonly mimeType: "image/png" | "image/jpeg" | "image/webp" | "image/avif";
  readonly byteLength: number;
  readonly width: number;
  readonly height: number;
}

export interface VnAudioDescriptor {
  readonly kind: "audio";
  readonly mimeType: "audio/wav" | "audio/mpeg" | "audio/ogg" | "audio/webm" | "audio/mp4";
  readonly byteLength: number;
  readonly durationMs: number;
  readonly sampleRate: number;
  readonly channels: number;
}

export type VnMediaDescriptor = VnImageDescriptor | VnAudioDescriptor;

export interface VnResolvedMedia<K extends MediaKind = MediaKind> {
  readonly status: "available";
  readonly kind: K;
  readonly role: VnMediaRole;
  readonly media: Readonly<MediaRef>;
  readonly revision: string;
  readonly variant: string;
  readonly descriptor: K extends "image" ? VnImageDescriptor : VnAudioDescriptor;
  readonly bytes: Uint8Array;
  /** Releases host-owned bytes or a temporary host lease after presentation has prepared them. */
  readonly release: () => void;
}

export interface VnUnavailableMedia {
  readonly status: "unavailable";
  readonly kind: MediaKind;
  readonly role: VnMediaRole;
  readonly media: Readonly<MediaRef>;
  readonly revision: string;
  readonly variant: string;
  readonly reason: "missing" | "authorization" | "temporary";
}

export type VnMediaResolution = VnResolvedMedia<"image"> | VnResolvedMedia<"audio"> | VnUnavailableMedia;
export type VnMediaResolver<HostContext> = (request: VnMediaRequest<HostContext>) => Promise<VnMediaResolution>;

/** Browser handles remain in the live player. The host must revoke/release them here. */
export type VnPresentedMedia =
  | {
      readonly kind: "image";
      readonly source: string;
      readonly release: () => void;
    }
  | {
      readonly kind: "audio";
      readonly playFromStart: (signal: AbortSignal) => Promise<void>;
      readonly stop: () => void;
      readonly release: () => void;
    };

export type VnMediaPresenter = (
  resource: VnResolvedMedia,
  request: Pick<VnMediaRequest<unknown>, "slot" | "variant" | "signal">,
  view: VnViewV1,
) => Promise<VnPresentedMedia>;

export interface VnMediaLimits {
  readonly maxRequests: number;
  readonly maxConcurrent: number;
  readonly maxEncodedBytesPerTarget: number;
  readonly maxEncodedBytesRetained: number;
  readonly maxDecodedBytesRetained: number;
  readonly maxImageDimension: number;
  readonly maxAudioDurationMs: number;
}

export const defaultVnMediaLimits: VnMediaLimits = Object.freeze({
  maxRequests: 8,
  maxConcurrent: 3,
  maxEncodedBytesPerTarget: 32 * 1024 * 1024,
  maxEncodedBytesRetained: 48 * 1024 * 1024,
  maxDecodedBytesRetained: 96 * 1024 * 1024,
  maxImageDimension: 8192,
  maxAudioDurationMs: 10 * 60 * 1000,
});

export class VnMediaBoundaryError extends Error {
  constructor(readonly issue: "role" | "kind" | "media" | "revision" | "variant" | "descriptor" | "budget") {
    super("The media host returned a resource that does not match the checked VN request.");
    this.name = "VnMediaBoundaryError";
  }
}

export class VnMediaUnavailableError extends Error {
  constructor(readonly role: VnMediaRole) {
    super("A required VN resource is unavailable.");
    this.name = "VnMediaUnavailableError";
  }
}

export interface VnPreparedAsset {
  readonly slot: VnMediaSlot;
  readonly media: Readonly<MediaRef>;
  readonly variant: string;
  readonly presentation: VnPresentedMedia;
  readonly encodedBytes: number;
  readonly decodedBytes: number;
}

export function expectedKind(role: VnMediaRole): MediaKind {
  return role === "background" || role === "character" ? "image" : "audio";
}

export function sameMedia(left: Readonly<MediaRef>, right: Readonly<MediaRef>): boolean {
  return left.kind === right.kind && left.provider === right.provider && left.key === right.key && left.revision === right.revision;
}

export function mediaSlotKey(slot: VnMediaSlot): string {
  return `${slot.role}:${slot.id}`;
}

export function mediaReuseKey(slot: VnMediaSlot, media: Readonly<MediaRef>, variant: string): string {
  return JSON.stringify([slot.role, media.kind, media.provider, media.key, media.revision, variant]);
}

export function variantId(value: unknown): value is string {
  return typeof value === "string" && /^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$/.test(value);
}

export function checkedVnMediaResolution(
  request: VnMediaRequest<unknown>,
  value: unknown,
  limits: VnMediaLimits,
): VnMediaResolution {
  const expectedRole = request.slot.role;
  const expectedMedia = request.slot.media;
  const requestKind = expectedKind(expectedRole);
  if (expectedMedia.kind !== requestKind || request.kind !== requestKind) throw new VnMediaBoundaryError("kind");
  if (!variantId(request.variant)) throw new VnMediaBoundaryError("variant");
  if (typeof value !== "object" || value === null || Array.isArray(value)) throw new VnMediaBoundaryError("descriptor");
  const result = value as Record<string, unknown>;
  if (result.status !== "available" && result.status !== "unavailable") throw new VnMediaBoundaryError("descriptor");
  const expectedKeys = result.status === "available"
    ? ["status", "kind", "role", "media", "revision", "variant", "descriptor", "bytes", "release"]
    : ["status", "kind", "role", "media", "revision", "variant", "reason"];
  if (!exactKeys(result, expectedKeys)) throw new VnMediaBoundaryError("descriptor");
  if (result.role !== expectedRole) throw new VnMediaBoundaryError("role");
  if (result.kind !== requestKind) throw new VnMediaBoundaryError("kind");
  if (result.revision !== expectedMedia.revision) throw new VnMediaBoundaryError("revision");
  if (result.variant !== request.variant) throw new VnMediaBoundaryError("variant");
  if (!sameMediaValue(result.media, expectedMedia)) throw new VnMediaBoundaryError("media");

  if (result.status === "unavailable") {
    if (!(result.reason === "missing" || result.reason === "authorization" || result.reason === "temporary")) {
      throw new VnMediaBoundaryError("descriptor");
    }
    return value as VnUnavailableMedia;
  }

  const descriptor = result.descriptor;
  const bytes = result.bytes;
  const release = result.release;
  if (typeof release !== "function" || !(bytes instanceof Uint8Array)) throw new VnMediaBoundaryError("descriptor");
  if (typeof descriptor !== "object" || descriptor === null || Array.isArray(descriptor)) throw new VnMediaBoundaryError("descriptor");
  const details = descriptor as Record<string, unknown>;
  const descriptorKeys = requestKind === "image"
    ? ["kind", "mimeType", "byteLength", "width", "height"]
    : ["kind", "mimeType", "byteLength", "durationMs", "sampleRate", "channels"];
  if (!exactKeys(details, descriptorKeys) || details.kind !== requestKind
    || !Number.isSafeInteger(details.byteLength) || details.byteLength !== bytes.byteLength
    || bytes.byteLength < 1 || bytes.byteLength > limits.maxEncodedBytesPerTarget) {
    throw new VnMediaBoundaryError("descriptor");
  }

  if (requestKind === "image") {
    if (typeof details.mimeType !== "string" || !isImageMime(details.mimeType)
      || !positiveSafe(details.width) || !positiveSafe(details.height)
      || details.width > limits.maxImageDimension || details.height > limits.maxImageDimension) {
      throw new VnMediaBoundaryError("descriptor");
    }
  } else if (typeof details.mimeType !== "string" || !isAudioMime(details.mimeType)
    || !positiveSafe(details.durationMs) || details.durationMs > limits.maxAudioDurationMs
    || !positiveSafe(details.sampleRate) || details.sampleRate < 8000 || details.sampleRate > 192000
    || !positiveSafe(details.channels) || details.channels > 8) {
    throw new VnMediaBoundaryError("descriptor");
  }
  return value as VnMediaResolution;
}

export function releaseVnMediaResolution(value: unknown): void {
  if (typeof value === "object" && value !== null && "release" in value && typeof value.release === "function") {
    try { value.release(); } catch { /* A malformed result still gets its cleanup attempt. */ }
  }
}

export function decodedMediaBytes(descriptor: VnMediaDescriptor): number {
  if (descriptor.kind === "image") return descriptor.width * descriptor.height * 4;
  return Math.ceil(descriptor.durationMs * descriptor.sampleRate * descriptor.channels * 4 / 1000);
}

function isImageMime(value: string): value is VnImageDescriptor["mimeType"] {
  return value === "image/png" || value === "image/jpeg" || value === "image/webp" || value === "image/avif";
}

function isAudioMime(value: string): value is VnAudioDescriptor["mimeType"] {
  return value === "audio/wav" || value === "audio/mpeg" || value === "audio/ogg" || value === "audio/webm" || value === "audio/mp4";
}

function positiveSafe(value: unknown): value is number {
  return typeof value === "number" && Number.isSafeInteger(value) && value > 0;
}

function sameMediaValue(value: unknown, expected: Readonly<MediaRef>): boolean {
  if (typeof value !== "object" || value === null || Array.isArray(value)) return false;
  const media = value as Record<string, unknown>;
  return exactKeys(media, ["kind", "provider", "key", "revision"])
    && media.kind === expected.kind && media.provider === expected.provider && media.key === expected.key && media.revision === expected.revision;
}

function exactKeys(value: Record<string, unknown>, expected: readonly string[]): boolean {
  const actual = Reflect.ownKeys(value);
  return actual.length === expected.length
    && actual.every(key => typeof key === "string" && expected.includes(key))
    && expected.every(key => Object.hasOwn(value, key));
}
