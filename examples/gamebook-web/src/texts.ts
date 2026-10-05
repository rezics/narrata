import type { BookView, ContentRef, GraphAnalysis, GraphRef, NodeAddress, NodeAnalysis, ViewScalar } from "@rezics/narrata";
import { contentKey, type Args, type Content, type Resolved } from "./protocol";

/** Stands in for content the provider did not resolve; the key helps authors find it. */
export function missing(content: Content): string { return `〔${"unit" in content ? content.unit.key : content.key}〕`; }

/** The texts the worker resolved for one view. */
export class Texts {
  constructor(private readonly entries: Record<string, Resolved>) {}

  resolved(content: Content, args?: Args): Resolved | undefined { return this.entries[contentKey(content, args)]; }

  /** A short text, or `fallback` for an absent or unresolved reference. */
  text(content: ContentRef | null | undefined, fallback: string, args?: Args): string {
    if (!content) return fallback;
    const value = this.resolved(content, args);
    return value?.ok ? value.blocks.join("\n\n") : fallback;
  }

  scalar(value: ViewScalar): string {
    switch (value.type) {
      case "bool": return value.value ? "是" : "否";
      case "int": case "text": return value.value;
      case "ref": return this.text(value.value, missing(value.value));
    }
  }
}

/** The page's heading: the last title presented, or the ending's. Items before it lead in. */
export function heading(book: BookView, texts: Texts): { index: number; text: string } {
  const interaction = book.view.interaction;
  if (interaction.kind === "finished") return { index: book.page.length, text: texts.text(interaction.title, "故事结束") };
  let index = -1;
  book.page.forEach((item, position) => { if (item.role === "title") index = position; });
  const item = book.page[index];
  if (item && !("unit" in item.content)) return { index, text: texts.text(item.content, missing(item.content), item.args) };
  const commit = book.view.history.find(entry => entry.current);
  return { index: -1, text: texts.text(commit?.title, commit?.key ?? "") };
}

export function graphTitle(graphs: GraphAnalysis[], reference: GraphRef, texts: Texts): string {
  const graph = graphs.find(g => g.reference.package === reference.package && g.reference.graph === reference.graph);
  return texts.text(graph?.title, `${reference.package}.${reference.graph}`);
}

export function nodeLabel(node: NodeAnalysis, graphs: GraphAnalysis[], texts: Texts): string {
  switch (node.kind) {
    case "passage": return texts.text(node.title, node.key ?? node.id);
    case "call": return node.callee ? `调用 ${graphTitle(graphs, node.callee, texts)}` : "调用";
    case "branch": return `条件 ${node.key ?? ""}`.trim();
    case "mutate": return `更新 ${node.key ?? ""}`.trim();
    case "return": return `返回 ${node.outcome ?? ""}`.trim();
  }
}

/** `package.graph.node`, with the node alias when the pack has a name table. */
export function nodeName(graphs: GraphAnalysis[], address: NodeAddress): string {
  const graph = graphs.find(g => g.reference.package === address.package && g.reference.graph === address.graph);
  const node = graph?.nodes.find(n => n.id === address.node);
  return `${address.package}.${address.graph}.${node?.key ?? address.node}`;
}
