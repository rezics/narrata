/** Immutable reader objects for one exact artifact. Content remains host-owned. */
export interface ArtifactSource {
  readonly manifest: Uint8Array;
  readonly names?: Uint8Array;
  readonly fetchChunk: (id: string) => Promise<Uint8Array>;
}
export type ArtifactLoader = (artifact: string) => Promise<ArtifactSource>;

/** A missing historical program/object is retryable and distinct from an invalid save. */
export class ArtifactUnavailable extends Error {
  constructor(readonly artifact: string, readonly object: string | undefined, options: ErrorOptions) {
    super(`Saved artifact is unavailable. Retry loading it.${options.cause instanceof Error ? ` ${options.cause.message}` : ""}`, options);
    this.name = "ArtifactUnavailable";
  }
}
