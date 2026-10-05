import { createGraphController, type GraphController, type GraphNode, type GraphOptions } from "./controller.js";
export * from "./wire.js";
export * from "./controller.js";

export interface MountGraphOptions extends GraphOptions { readonly onSelect?: (node: GraphNode) => void }
/** Dispose releases WebGL contexts, subscriptions and this view's bounded object cache. */
export function mountAuthorGraph(container: HTMLElement, options: MountGraphOptions): { readonly controller: GraphController; readonly ready: Promise<void>; dispose(): void } {
  const controller = createGraphController(options);
  let disposed = false;
  let renderer: Awaited<ReturnType<typeof import("./renderer.js")["createRenderer"]>> | undefined;
  const select = (node: GraphNode) => {
    if (node.enter) void (node.enter.partition === undefined ? controller.enterCluster(node.enter.cluster) : controller.enterPartition(node.enter.cluster, node.enter.partition));
    else options.onSelect?.(node);
  };
  const render = () => { const screen = controller.getSnapshot().screen; if (screen) renderer?.render(screen); };
  const unsubscribe = controller.subscribe(render);
  const ready = (async () => {
    const module = await import("./renderer.js");
    if (disposed) return;
    renderer = module.createRenderer(container, select);
    await controller.start();
    render();
  })();
  return { controller, ready, dispose() { disposed = true; unsubscribe(); renderer?.dispose(); controller.dispose(); } };
}
