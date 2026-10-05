import Sigma from "sigma";
import { MultiDirectedGraph } from "graphology";
import type { GraphNode, GraphScreen } from "./controller.js";

/** This module is imported only when a host mounts the author view. Never runs a layout. */
export function createRenderer(container: HTMLElement, select: (node: GraphNode) => void) {
  const graph = new MultiDirectedGraph();
  const sigma = new Sigma(graph, container, { allowInvalidContainer: true, renderEdgeLabels: false });
  let nodes = new Map<string, GraphNode>();
  let previous: GraphScreen | undefined;
  sigma.on("clickNode", event => { const node = nodes.get(event.node); if (node) select(node); });
  return {
    render(screen: GraphScreen) {
      if (previous === screen) return;
      const sameGeometry = previous && previous.level === screen.level && previous.cluster === screen.cluster && previous.partition === screen.partition
        && previous.edges === screen.edges && previous.nodes.length === screen.nodes.length;
      nodes = new Map(screen.nodes.map(node => [node.id, node]));
      if (sameGeometry) for (const node of screen.nodes) graph.setNodeAttribute(node.id, "label", node.label);
      else {
        graph.clear();
        for (const node of screen.nodes) graph.addNode(node.id, { x: node.x, y: -node.y, label: node.label, size: screen.level === "node" ? 4 : 8, color: node.ghost ? "#aaa" : "#845c39" });
        for (const edge of screen.edges) graph.addDirectedEdgeWithKey(edge.id, edge.source, edge.target, { size: 1, color: edge.mask & 8 ? "#845c39" : "#b6b4ab" });
        sigma.getCamera().setState({ x: 0.5, y: 0.5, ratio: 1, angle: 0 });
      }
      previous = screen;
      sigma.refresh();
    },
    dispose: () => { sigma.kill(); graph.clear(); },
  };
}
