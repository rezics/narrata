import Sigma from "sigma";
import { MultiDirectedGraph } from "graphology";
import type { GraphNode, GraphScreen } from "./controller.js";
import { createLabelPlacement, graphCanvasLabel, projectionGroups } from "./labels.js";

/** This module is imported only when a host mounts the author view. Never runs a layout. */
export function createRenderer(container: HTMLElement, select: (node: GraphNode) => void) {
  const graph = new MultiDirectedGraph();
  let placeLabel = createLabelPlacement(container.clientWidth, container.clientHeight);
  let projected = projectionGroups([]);
  const drawnCounts = new Set<string>();
  const sigma = new Sigma(graph, container, { allowInvalidContainer: true, renderEdgeLabels: false,
    stagePadding: 45, labelSize: 13, labelRenderedSizeThreshold: 0,
    defaultDrawNodeLabel(context, data, settings) {
      if (!data.label || !data.key) return;
      context.font = `${settings.labelWeight} ${settings.labelSize}px ${settings.labelFont}`;
      const group = projected.get(data.key);
      if (group && group.count > 1 && !drawnCounts.has(group.key)) {
        drawnCounts.add(group.key);
        context.strokeStyle = "#193e37"; context.lineWidth = 1;
        context.fillStyle = "#faf9f6";
        context.beginPath(); context.arc(data.x, data.y, data.size + 3, 0, Math.PI * 2); context.fill(); context.stroke();
        context.font = `bold 11px ${settings.labelFont}`;
        context.fillStyle = "#193e37"; context.textAlign = "center"; context.textBaseline = "middle";
        context.fillText(String(group.count), data.x, data.y);
        context.font = `${settings.labelWeight} ${settings.labelSize}px ${settings.labelFont}`;
      }
      let label = graphCanvasLabel(data.label, data.key);
      const maximum = Math.min(240, container.clientWidth - 16);
      if (context.measureText(label).width > maximum) {
        const characters = Array.from(label);
        while (characters.length && context.measureText(`${characters.join("")}…`).width > maximum) characters.pop();
        label = `${characters.join("")}…`;
      }
      const box = placeLabel(data.x, data.y, data.size, context.measureText(label).width, settings.labelSize + 4);
      if (!box) return;
      context.fillStyle = "#193e37";
      context.textAlign = "left"; context.textBaseline = "middle";
      context.fillText(label, box.left, (box.top + box.bottom) / 2);
    },
  });
  sigma.on("beforeRender", () => {
    const { width, height } = sigma.getDimensions(); placeLabel = createLabelPlacement(width, height);
    drawnCounts.clear();
    // Small views can label every node and mark subpixel coincidences without moving geometry.
    projected = nodes.size <= 200 ? projectionGroups([...nodes.values()].map(node => ({ id: node.id, ...sigma.graphToViewport({ x: node.x, y: -node.y }) }))) : projectionGroups([]);
  });
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
        for (const node of screen.nodes) graph.addNode(node.id, { x: node.x, y: -node.y, label: node.label, forceLabel: screen.nodes.length <= 200, size: screen.level === "node" ? 5 : 8, color: node.ghost ? "#8a9a94" : "#845c39" });
        for (const edge of screen.edges) graph.addDirectedEdgeWithKey(edge.id, edge.source, edge.target, { size: 1.25, color: edge.mask & 8 ? "#845c39" : "#8a9a94" });
        sigma.getCamera().setState({ x: 0.5, y: 0.5, ratio: 1, angle: 0 });
      }
      previous = screen;
      sigma.refresh();
    },
    dispose: () => { sigma.kill(); graph.clear(); },
  };
}
