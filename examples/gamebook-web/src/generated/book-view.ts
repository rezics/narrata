/* Generated from the Rust BookView JSON Schema. Run npm run prepare:runtime. */

export type ScalarType = "bool" | "int" | "text";

/**
 * Authoritative read model shared by browser transports and generated TypeScript validation.
 */
export interface BookView {
  diagnostics: Diagnostic[];
  graphs: GraphAnalysis[];
  view: SessionView;
}
export interface Diagnostic {
  code: string;
  message: string;
  path: string;
}
export interface GraphAnalysis {
  entry: string;
  nodes: NodeAnalysis[];
  reference: GraphRef;
  title: string;
}
export interface NodeAnalysis {
  edges: EdgeAnalysis[];
  id: string;
  label: string;
  type_id: string;
}
export interface EdgeAnalysis {
  kind: string;
  label: string;
  target: NodeAddress;
}
export interface NodeAddress {
  graph: string;
  node: string;
  package: string;
}
export interface GraphRef {
  graph: string;
  package: string;
}
export interface SessionView {
  actions: ActionView[];
  artifact_id: string;
  cursor: string;
  finished: boolean;
  frames: FrameView[];
  history: HistoryView[];
  instance: number;
  node: NodeAddress;
  outcome?: string | null;
  paragraphs: string[];
  product_title: string;
  shared: VariableView[];
  title: string;
}
export interface ActionView {
  enabled: boolean;
  id: string;
  label: string;
  reason?: string | null;
}
export interface FrameView {
  graph: GraphRef;
  instance: number;
  locals: VariableView[];
  node: string;
  parameters: VariableView[];
}
/**
 * Scalar payloads are formatted strings, so 64-bit integers never lose precision in JavaScript.
 */
export interface VariableView {
  kind: ScalarType;
  label: string;
  name: string;
  value: string;
}
export interface HistoryView {
  current: boolean;
  id: string;
  instance: number;
  node: NodeAddress;
  parent?: string | null;
  title: string;
}
