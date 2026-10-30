import { useEffect, useRef, type CSSProperties } from "react";
import { mountAuthorGraph, type MountGraphOptions } from "./index.js";
import type { GraphController } from "./controller.js";

export interface AuthorGraphProps {
  /** Keep options stable; a new identity replaces the mounted view and its cache. */
  readonly options: MountGraphOptions;
  readonly onController?: (controller: GraphController | null) => void;
  readonly onError?: (error: Error) => void;
  readonly className?: string;
  readonly style?: CSSProperties;
  readonly label?: string;
}
/** Optional React peer, matching the reader component convention. Give the container a height. */
export function AuthorGraph({ options, onController, onError, className, style, label = "Author graph" }: AuthorGraphProps) {
  const container = useRef<HTMLDivElement>(null);
  const callbacks = useRef({ onController, onError });
  callbacks.current = { onController, onError };
  useEffect(() => {
    if (!container.current) return;
    const mounted = mountAuthorGraph(container.current, options);
    callbacks.current.onController?.(mounted.controller);
    void mounted.ready.catch(error => callbacks.current.onError?.(error instanceof Error ? error : new Error(String(error))));
    return () => { mounted.dispose(); callbacks.current.onController?.(null); };
  }, [options]);
  return <div ref={container} className={className} style={style} role="img" aria-label={label} />;
}
