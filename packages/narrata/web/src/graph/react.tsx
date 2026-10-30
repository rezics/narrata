import { useEffect, useRef, type CSSProperties } from "react";
import { mountAuthorGraph, type MountGraphOptions } from "./index.js";
import type { GraphController } from "./controller.js";
import type { ReaderMapV1 } from "../generated/authoring.js";

/** A revealed map needs no program, author tiles, labels table, trace or layout download. */
export function RevealedReaderMap({ map }: { readonly map: ReaderMapV1 }) {
  const nodes = new Map(map.nodes.map(node => [node.node, node]));
  const height = Math.max(100, ...map.nodes.map(node => (node.y + 1) * 80));
  return <section aria-label="Revealed story map" data-testid="reader-map" style={{ marginTop: 24, maxWidth: "100%" }}>
    <h2>Your revealed map</h2>
    <p>{map.sources.every(source => source.complete) ? "This map follows your saved journey." : "Some history is unavailable. This map shows only verified visits."}</p>
    <svg viewBox={`0 0 640 ${height}`} role="img" aria-label="Visited story connections" style={{ width: "100%", maxHeight: 400 }}>
      {map.edges.map((edge, index) => { const a = nodes.get(edge.source), b = nodes.get(edge.target); return a && b ? <line key={index} x1={a.x * 76 + 36} y1={a.y * 80 + 40} x2={b.x * 76 + 36} y2={b.y * 80 + 40} stroke="currentColor" /> : null; })}
      {map.nodes.map((node, index) => <g key={node.node}><circle cx={node.x * 76 + 36} cy={node.y * 80 + 40} r={18} fill="#e7eadb" stroke="#3c5246" /><text x={node.x * 76 + 36} y={node.y * 80 + 45} textAnchor="middle" fontSize={13}>{index + 1}</text></g>)}
    </svg>
    <p>{map.units.length} revealed units · {map.endings.length} reached endings</p>
    <ul aria-label="Visible options">{map.sources.flatMap(source => source.frontier).map((option, index) => <li key={`${option.option}:${index}`}>{`Visible option ${index + 1}`}{option.enabled ? "" : " (unavailable)"}</li>)}</ul>
  </section>;
}

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
