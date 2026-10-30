import type { ReaderMapV1 } from "../generated/authoring.js";
export type { ReaderMapV1 } from "../generated/authoring.js";

/** A revealed map needs no program, author tiles, labels table, trace or layout download. */
export function RevealedReaderMap({ map }: { readonly map: ReaderMapV1 }) {
  const nodes = new Map(map.nodes.map(node => [node.node, node]));
  const height = map.nodes.reduce((height, node) => Math.max(height, (node.y + 1) * 80), 100);
  return <section aria-label="Revealed story map" data-testid="reader-map" style={{ marginTop: 24, maxWidth: "100%" }}>
    <h2>Your revealed map</h2>
    <p>{map.sources.every(source => source.complete) ? "This map follows your saved journey." : "Some history is unavailable. This map shows only verified visits."}</p>
    <div role="region" aria-label="Scrollable revealed map" tabIndex={0} style={{ overflow: "auto", maxWidth: "100%", maxHeight: 400 }}>
    <svg width={640} height={height} viewBox={`0 0 640 ${height}`} role="img" aria-label="Visited story connections" style={{ display: "block", width: 640, minWidth: 640, height }}>
      {map.edges.map((edge, index) => { const a = nodes.get(edge.source), b = nodes.get(edge.target); return a && b ? <line key={index} x1={a.x * 76 + 36} y1={a.y * 80 + 40} x2={b.x * 76 + 36} y2={b.y * 80 + 40} stroke="currentColor" /> : null; })}
      {map.nodes.map((node, index) => <g key={node.node} data-map-node={index + 1}><title>{`Visited point ${index + 1}`}</title><circle cx={node.x * 76 + 36} cy={node.y * 80 + 40} r={18} fill="#e7eadb" stroke="#3c5246" /><text x={node.x * 76 + 36} y={node.y * 80 + 45} textAnchor="middle" fontSize={16}>{index + 1}</text></g>)}
    </svg>
    </div>
    <p>{map.units.length} revealed units · {map.endings.length} reached endings</p>
    <ul aria-label="Visible options">{map.sources.flatMap(source => source.frontier).map((option, index) => <li key={`${option.option}:${index}`}>{`Visible option ${index + 1}`}{option.enabled ? "" : " (unavailable)"}</li>)}</ul>
  </section>;
}

