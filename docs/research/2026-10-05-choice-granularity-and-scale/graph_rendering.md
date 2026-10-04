# Rendering and Exploring 100k–300k-Node Narrative Graphs in the Browser: Libraries, Layout, Exploration Patterns, Reader Path Maps, Data Delivery

Scope note: these notes cover how to show the narrative graph of a Narrata work in a web client: a directed multigraph of passages and choice edges, ~100k–300k nodes and ~300k–900k edges, mostly DAG-like with merges and some cycles. The client is React 19 on Cloudflare Workers with a ~330 KB (compressed) per-page JS budget, so heavy code has to be lazy-loaded. Two audiences: **authors** (edit and analyse the whole work) and **readers** (a spoiler-safe map of the paths they have explored).

Research date: 2026-10-05. Versions and dates come from the npm registry on that day. A claim taken from a search-engine excerpt, a vendor page, or a community post rather than a primary document is marked **[secondary]**, **[vendor]** or **[community]**. Numbers marked **[measured here]** come from benchmarks run for these notes. Method and caveats are in Appendix A; the scripts were throwaway scratchpad files and are not committed.

---

## 0. Takeaways (one screen)

1. **No browser library makes "draw all 300k nodes + 900k edges with labels and editing" a good experience on ordinary hardware.** On an Intel UHD iGPU we measured **sigma.js v3** at 60 fps for 30k/89k, 41–60 fps for 100k/300k, and **17–29 fps for 300k/900k**, with **+456 MB of JS heap**. **cosmos.gl v3** held **40 fps for 300k/900k with +220 MB**, but has no built-in labels or editing model. **[measured here]** Every credible system for graphs this size bounds what the client draws: aggregation, LOD and tiling (GraphMaps, Ogma, yFiles, Graphistry).
2. **Layered (Sugiyama) layout does not scale in the browser JS libraries.** dagre with the default network-simplex ranker took 52 s at 1k nodes, and dagre threw a geometry error on our graphs at ≥2k nodes (cyclic) and ≥10k (acyclic). elkjs took ~10 s and 1.9 GB of heap at 5k nodes on fast settings, and did not finish 10k in 400 s. d3-dag "fast" took 132 s at 50k and 563 s (1.9 GB) at 100k. **[measured here]** Graphviz `dot` on a 353k-node graph spent 4,715 s in mincross alone ([retdec#604](https://github.com/avast/retdec/issues/604)). A purpose-built linear pipeline is a different class: a crude JS version laid out 300k/900k in 0.19 s **[measured here]**, and ArmoniK's custom layout did 12,472 tasks in 183 ms where ELK had not finished after 25 min ([ArmoniK PR #1379](https://github.com/aneoconsulting/ArmoniK.Admin.GUI/pull/1379)). **So precomputed (offline or server-side) layout is the norm at 100k+, and it should be a hierarchical, per-cluster layout.**
3. **Narrative tools already handle scale through nesting, not rendering.** articy:draft nests flows and has you "submerge" into them. Yarn Spinner groups nodes into file containers and clusters. Arcweave uses boards and jumpers. Twine's flat story map lags at ~500–800 passages, and its users split stories across files. Disco Elysium's ~1M-word script reportedly froze articy.
4. **Reader path maps in games are per-chapter and fog-of-war.** Detroit: Become Human and As Dusk Falls show per-chapter flowcharts with locked or masked nodes and community percentages. Zero Escape and AI: The Somnium Files use flowcharts with jump-to and lock icons. YU-NO's A.D.M.S. shows only discovered branches. Spoiler safety has to be enforced **server-side**: never ship unrevealed nodes to the client.
5. **Data volume is not the bottleneck; the in-memory object model is.** In our synthetic test, 300k/900k came to ~2.3 MB brotli as typed-array/Arrow columns. The same data takes ~390 MB as a graphology object graph in V8 and ~11 MB as typed arrays **[measured here]**.

---

## Q1. Rendering libraries: versions, maintenance, practical limits

### Takeaway
There are three tiers. **DOM/SVG** (React Flow, D3-SVG) is good to hundreds and up to ~1k rich nodes. **Canvas 2D** (Cytoscape canvas, vis-network, D3-canvas, G6 canvas) is good to a few thousand nodes and tens of thousands of edges. **WebGL** (sigma.js, cosmos.gl, Cytoscape's WebGL preview, G6 WebGL, deck.gl, Ogma, yFiles WebGL2) is good to ~10^5 elements, with frame rate depending on the GPU and on overdraw. For our budget and needs, the best fit for an editable author view is **sigma.js v3 + graphology** (MIT, ~37 KB gz). **cosmos.gl** (MIT, ~189 KB gz) is the strongest option for a "whole-work galaxy" view of every point. G6 (~433 KB gz), elkjs (~430 KB gz), deck.gl (~240 KB gz) and the Graphviz WASM builds (~480–620 KB gz) cannot fit the page budget and would have to live in lazy chunks or workers.

### Comparison matrix

Bundle sizes are **[measured here]**: esbuild ESM bundle of the named entry points, minified, then gzip -9 / brotli q11. Real app tree-shaking may differ.

| Library (npm) | Latest version (date) | License | Renderer | min / gzip / brotli | Practical scale evidence | Status notes |
|---|---|---|---|---|---|---|
| sigma + graphology | sigma 3.0.3 (2026-04-30), v4 beta 4.0.0-beta.7 (2026-10-02); graphology 0.26.0 (2025-01) | MIT | WebGL | 156 KB / **37 KB** / 32 KB (+FA2 worker: 41 KB gz) | Measured: 100k/300k at 60 fps (RTX 4080), 41–60 fps (Intel UHD, short edges); 300k/900k at 53 fps (dGPU), 17–29 fps (iGPU). Vendor claim: "100k edges easily", "struggles with 5k nodes with icons", FA2 falters past 50k edges ([Ogma compare page](https://doc.linkurious.com/ogma/latest/compare/sigmajs.html)) **[vendor]** | Stewarded by OuestWare. v3 stable Dec 2024 ([roadmap](https://github.com/jacomyal/sigma.js/discussions/1469)). v4 moves all rendering, labels included, to WebGL with OIT; alpha Apr 2026, maintainer expected stable ~Oct 2026 ([v4 discussion](https://github.com/jacomyal/sigma.js/discussions/1539)) |
| @cosmos.gl/graph | 3.4.2 (2026-09-21) | MIT | WebGL2 via luma.gl; GPU force sim | 708 KB / **189 KB** / 156 KB | Measured: 300k/900k at 60 fps (dGPU) and 40 fps (iGPU), +220 MB heap. Project claims "hundreds of thousands of points and links" in real time ([repo](https://github.com/cosmosgl/graph)) and 1M+ ([OpenJS](https://openjsf.org/blog/introducing-cosmos-gl)) | OpenJS Foundation incubating. v3 (2026-06-25) moved to luma.gl and added transitions, touch and link events ([v3 post](https://openjsf.org/blog/cosmos-gl-v3)). Data is `Float32Array`; `enableSimulation:false` accepts precomputed positions. No built-in labels; `getSampledLinks()` / overlays instead. Needs float-texture extensions (iOS/Android caveats in README) |
| @cosmograph/cosmograph | 2.5.1 (2026-08-14) | **CC-BY-NC-4.0** | cosmos + DuckDB-wasm + Arrow | (not measured) | Same engine as above | **Not usable commercially without a business license** ([licensing](https://cosmograph.app/licensing/)) |
| cytoscape | 3.34.3 (2026-09-07) | MIT | Canvas; WebGL **preview** since 3.31 | 435 KB / **140 KB** / 118 KB | 1.2k nodes/16k edges: 20 fps canvas → 100+ fps WebGL. 3.2k/68k: 3 → 10 fps (M1, Chrome) ([Cytoscape blog](https://blog.js.cytoscape.org/2025/01/13/webgl-preview/)) | WebGL is still labelled "preview" (no newer status post through 3.33). Limits: no dashed edges, triangle arrows only, centered edge labels only. Strengths: compound nodes, rich styling, analysis API |
| @antv/g6 | 5.1.1 (2026-05-08) | MIT | Canvas/SVG/WebGL (`@antv/g-webgl`), per-layer | 1,488 KB / **433 KB** / 352 KB | Paper: G6 beats Cytoscape at 4,331–35,000 elements, and Sigma "fails" at 13,744 and 35,000 ([Wang et al. 2021, Visual Informatics](https://www.sciencedirect.com/science/article/pii/S2468502X21000619)) **[secondary: search excerpt]**. Docs recommend `optimize-viewport-transform` above ~500 elements ([doc](https://g6.antv.antgroup.com/manual/behavior/optimize-viewport-transform)) | Ant Group. Combos and collapse built in; Rust/WASM and WebGPU layouts (`@antv/layout` 2.0.0, 2026-02). Too heavy for our page budget |
| @xyflow/react (React Flow) | 12.12.0 (2026-09-24) | MIT | DOM (React) | **60 KB** gz excl. React | Maintainer: "not intended to be used in that kind of scale" (1000+ nodes) ([discussion #3003](https://github.com/xyflow/xyflow/discussions/3003)). Official perf guide: memoize, avoid subscribing to `nodes`, hide collapsed subtrees ([docs](https://reactflow.dev/learn/advanced-use/performance)) | Well maintained. Right tool for **small** rich editors (one scene or hub), not for the map |
| vis-network | 10.1.2 (2026-08-19) | Apache-2.0 OR MIT | Canvas | 648 KB / 155 KB / 130 KB | Memgraph benchmark: 1k nodes/1k edges took 45 s–1:14 min to stabilise; sigma took 2 s ([Memgraph 2022](https://memgraph.com/blog/you-want-a-fast-easy-to-use-and-popular-graph-visualization-tool)). Has a clustering API (`initialMaxNodes`, `clusterThreshold`) for ">50,000 nodes" ([old docs](https://www.hlt.inesc-id.pt/~david/wiki/pt/extensions/vis/docs/network.html)) **[secondary]** | Community-maintained; physics-first |
| Ogma (Linkurious) | 6.0.9 (2026-07-09) ([changelog](https://doc.linkurious.com/ogma/latest/changelog.html)) | Commercial | WebGL (+Canvas/SVG) | n/a | Vendor: "100,000+ nodes like a breeze", layout of 1M+ edges ([compare](https://doc.linkurious.com/ogma/latest/compare/sigmajs.html)) **[vendor]**. GPU layout only <7,000 nodes; "One character uses as much space as a node shape" ([best practices](https://doc.linkurious.com/ogma/latest/tutorials/best-practices/)) | First-party advice is useful even without buying: "Filter your graph on the back end" |
| yFiles for HTML | 3.x | Commercial | SVG/Canvas/WebGL2 | n/a | WebGL2 "more than 20,000 elements". A ten-thousand-node layout takes "at least half a minute" ([large-graph guide](https://docs.yfiles.com/yfiles-html/dguide/large_graph_performance/)) | Best-in-class incremental `HierarchicalLayout` (keeps existing coordinates) |
| deck.gl (+ `@deck.gl-community/graph-layers`) | deck.gl 9.4.0 (2026-09-05); graph-layers 9.4.1 | MIT | WebGL2/WebGPU via luma.gl | core+Scatterplot/Line/Text: 854 KB / 240 KB / 199 KB | ScatterplotLayer ~1M points interactive **[secondary]**. The graph-layers README is literally "TBD" ([README](https://raw.githubusercontent.com/visgl/deck.gl-community/master/modules/graph-layers/README.md)) | A reasonable base for a custom renderer; graph-layers itself is immature |
| D3 (d3-force, SVG/canvas) | d3 7.9.0 (2024-03) | ISC | SVG/Canvas (you build it) | n/a | Memgraph: D3-canvas took 27 s to settle 10k/10k ([Memgraph](https://memgraph.com/blog/you-want-a-fast-easy-to-use-and-popular-graph-visualization-tool)). Rule of thumb: SVG ~2k elements, canvas ~5k, WebGL ~10k+ ([gdotv](https://gdotv.com/blog/practical-advice-on-graph-visualization-with-sigmajs/); [2025 efficiency study](https://www.researchgate.net/publication/391575870_Graph_visualization_efficiency_of_popular_web-based_libraries)) **[secondary]** | Stable but slow-moving. Use for small bespoke reader visuals |
| MSAGL-JS (`@msagl/*`) | 1.1.24–1.1.26 (2026-04) | MIT | WebGL (deck.gl) / SVG | (not measured) | Tile-pyramid browsing: 95% of frame gaps ≤18.4 ms on graphs ≤32k nodes/237k edges. Pyramid build 14–23 s for the larger graphs ([Nachmanson & Chen 2026](https://arxiv.org/html/2605.17498)) | Research-grade, but the only open implementation of map-style graph tiles |

### Cited findings

- **sigma.js** describes itself as "aimed at visualizing graphs of thousands of nodes and edges". It recommends D3 for small, heavily customized graphs, and leaves layout (e.g. ForceAtlas2) to graphology ([sigmajs.org](https://www.sigmajs.org/)). Rendering uses instanced WebGL "programs". `NodePointProgram` and `EdgeLineProgram` are the cheapest (1-px lines). Picking is done by drawing an offscreen ID-color image ([renderers doc](https://www.sigmajs.org/docs/advanced/renderers/)).
- sigma v3 performance settings, from the installed package source: `hideEdgesOnMove` (false), `hideLabelsOnMove` (false), `enableEdgeEvents` (false; edge picking costs extra), `labelDensity` 1, `labelGridCellSize` 100, `labelRenderedSizeThreshold` 6. `nodeReducer`/`edgeReducer` restyle without mutating the graph. `refresh({ partialGraph: { nodes, edges }, skipIndexation })` re-processes only touched items, which matters for editing. **[measured here: read from `sigma@3.0.3` d.ts and source]**
- sigma v4 (alpha 2026-04-23, betas through 2026-10-02) addresses v3's layering problems: "Migrate all rendering to WebGL (no more canvas)". It brings WebGL-rendered labels with picking and events, and fewer WebGL contexts. The maintainer says APIs "should not move any longer" and expects stable ~first half of Oct 2026 ([discussion #1539](https://github.com/jacomyal/sigma.js/discussions/1539); npm `sigma` dist-tags).
- **cosmos.gl**: "all the computations and drawing occur on the GPU in fragment and vertex shaders" ([OpenJS intro](https://openjsf.org/blog/introducing-cosmos-gl)). The GPU many-body approximation runs on a square grid; dense cells add noise, and very large graphs can run out of `spaceSize` ([Nightingale, Rokotyan 2022](https://nightingaledvs.com/how-to-visualize-a-graph-with-a-million-nodes/)). CPU force layouts "hit performance walls around 50,000 nodes" ([Cosmograph concept](https://cosmograph.app/docs-general/concept/)).
- **Cytoscape.js WebGL**: node bodies are drawn to an offscreen "sprite sheet" with the canvas renderer and used as textures by WebGL. This explains why all node styles are supported but edge styles are restricted ([blog](https://blog.js.cytoscape.org/2025/01/13/webgl-preview/)). Enable with `renderer: { name: 'canvas', webgl: true }`.
- **Graphistry** (server-GPU + browser client): "a 10-million-edge graph that needed 7.6 GB of browser memory now needs 537 MB", i.e. ≈54 bytes per edge all-in. "most of that 7.6 GB was the precomputation, not the graph." Interaction frame time on a 1.6M-edge graph dropped from 83 ms to 29 ms ([Graphistry 2.53.0, 2026-09-03](https://www.graphistry.com/blog/graphistry-2-53-0-large-graph-visualization-at-10-million-edges)) **[vendor]**. Lesson: keep CPU-side per-edge objects out of the client.
- A related report from a Cytoscape app at 1,600 entities / 9,000 links: "Cytoscape element reconciliation rather than neighborhood computation" dominated focus-expansion time ([Codex-Cryptica#3160](https://github.com/eserlan/Codex-Cryptica/issues/3160)) **[community]**. **The library's object model, not the GPU, is often the real cost.**

### Our rendering benchmark **[measured here]**

Setup: Chrome stable on Windows 11, a 1,600×900 canvas, vsync 60 Hz, **precomputed positions** (no layout), nodes size 2, edges 0.5, camera animated every frame for 4 s. "Out" = whole graph in view; "in" = 20× zoom. Each graph has ~3 edges per node.

| Library | Graph | GPU | Edge geometry | Build (ms) | First frame (ms) | JS heap Δ | fps out / in (p95 frame ms) |
|---|---|---|---|---|---|---|---|
| sigma 3.0.3 | 100k / 300k | RTX 4080 Laptop | random long | 542 | 382 | +150 MB | 60 / 60 (21 / 20) |
| sigma 3.0.3 | 300k / 900k | RTX 4080 Laptop | random long | 2,646 | 1,318 | +456 MB | 53 / 53 (24 / 21) |
| cosmos.gl 3.4.2 | 100k / 300k | RTX 4080 Laptop | random long | 5 | 432 | +83 MB | 60 / 60 |
| cosmos.gl 3.4.2 | 300k / 900k | RTX 4080 Laptop | random long | 7 | 560 | +220 MB | 60 / 60 |
| sigma | 30k / 89k | Intel UHD (iGPU) | random long | 101 | 200 | +49 MB | 23 / 58 (50 / 17) |
| sigma | 50k / 149k | Intel UHD | random long | 211 | 234 | +75 MB | 15 / 57 |
| sigma | 100k / 300k | Intel UHD | random long | 546 | 405 | +150 MB | 19 / 35 |
| sigma | 300k / 900k | Intel UHD | random long | 2,441 | 1,284 | +456 MB | 11 / 12 (97 / 89) |
| cosmos.gl | 300k / 900k | Intel UHD | random long | 6 | 559 | +220 MB | 40 / 42 |
| sigma | 30k / 89k | Intel UHD | **short, ordered** | 90 | 176 | +49 MB | **60 / 60** |
| sigma | 100k / 300k | Intel UHD | short, ordered | 561 | 482 | +150 MB | **41 / 60** |
| sigma | 300k / 900k | Intel UHD | short, ordered | 2,449 | 1,331 | +457 MB | **17.5 / 29** |
| cosmos.gl | 100k / 300k | Intel UHD | short, ordered | 6 | 344 | +83 MB | 60 / 60 |
| cosmos.gl | 300k / 900k | Intel UHD | short, ordered | 10 | 572 | +219 MB | **40 / 40** |

Interpretation:
- On an iGPU, **zoomed-out frame rate is driven by edge overdraw**. Long criss-crossing edges (a "hairball") cut sigma's 30k-node frame rate from 60 to 23 fps. A good layered layout (short edges) is itself a performance feature.
- **A drawable budget of ~30–50k nodes / ~100–150k edges per view** holds 60 fps on weak GPUs with sigma. Larger working sets need LOD or aggregation (see Q3).
- **sigma's CPU cost scales with graphology**: ~2.5 s to build and ~1.3 s to process 300k/900k, plus ~0.45 GB of heap. That is acceptable on a desktop for a "load chapter" action, but not for the whole work on a phone. cosmos.gl ingests typed arrays directly (5–10 ms) and uses about half the heap, but gives up labels, an editing model and rich styling.

---

## Q2. Layout at 10k–300k nodes: layered vs force, offline vs online, incremental stability

### Takeaway
- **Sugiyama/layered layout is what fits narrative DAGs** (time flows one way, merges stay visible, cycles are drawn as back edges). The standard JS implementations, however, are **super-linear and memory-hungry**. Crossing minimisation and coordinate assignment (network simplex / QP) dominate, and long edges expand into dummy nodes.
- **At 100k+ nodes, precomputed layout is the norm**: Graphistry's server GPUs, GraphMaps' up-front tile pyramid, Dagster's IndexedDB layout cache, and Ogma's "filter on the back end". Two practical strategies are custom linear-time pipelines (ArmoniK) and **hierarchical decomposition**: lay out each chapter separately, then pack the chapters.
- **Force-directed layouts** (FA2, cosmos.gl GPU) scale to 10^5–10^6 nodes. But they are non-deterministic, ignore the story's direction, and drift when the graph changes. They suit an analysis "galaxy" view, not the primary story map.
- **Stability under edits** comes from: (a) persisting coordinates and ordering keys; (b) the "interactive" modes of ELK, yFiles and dynadag that respect the previous layout; (c) laying out only the affected cluster.

### Cited findings: library scaling

| Engine | Evidence |
|---|---|
| **Graphviz dot** | Call graph of **353,568 nodes / 536,598 edges**: mincross **4,714.91 s**, network simplex 852.99 s over 16,366 iterations, splines 89.02 s. ~1.5 h total with Graphviz 2.41; output unreadable ([retdec#604](https://github.com/avast/retdec/issues/604)). The forum suggests `-Gnslimit=2 -Gnslimit1=2 -Gmaxiter=5000` for speed ([forum](https://forum.graphviz.org/t/where-does-generating-the-graph-take-most-of-the-time/668)). WASM builds exist (`@viz-js/viz` 3.31.0 MIT, `@hpcc-js/wasm-graphviz` 1.29.2 Apache-2.0) but cost **478 KB and 617 KB gz** **[measured here]** |
| **dagre** (`@dagrejs/dagre` 3.1.1, 2026-08; the old `dagre` 0.8.5 is from 2019) | Dagster: the default "network-simplex" ranker is O(N³) worst case. Switching to "tight-tree" took a 2,000-asset graph from minutes to ~5 s, plus an IndexedDB layout cache ([Dagster 2023](https://dagster.io/blog/scaling-dag-visualization)). Users report 1,300 nodes blocking the browser for seconds and 1.7k/7k freezing Chrome ([dagre#315](https://github.com/dagrejs/dagre/issues/315)) **[community]** |
| **elkjs** 0.12.0 (2026-07), EPL-2.0 OR GPL-3.0+ | ELK paper: "elkjs is not as performant for larger graphs as the original ELK" (GWT-transpiled Java) ([Domrös et al.](https://arxiv.org/html/2311.00533v1)). ArmoniK: 10,000 independent tasks took 2,057 ms with ELK vs 36 ms custom; a 12,472-task map-reduce had **not finished after 25 minutes** in ELK vs 183 ms custom. Their custom WebGL2 renderer ran **60 fps at 30k nodes** vs 2–10 fps for force-graph ([PR #1379](https://github.com/aneoconsulting/ArmoniK.Admin.GUI/pull/1379)). Another app hit ~4.6 s at 500 nodes and ~33 s at 1,000 **[community, search excerpt]** |
| **d3-dag** 1.2.2 (2026-07) | Docs: "fast" preset 5.1 ms vs "medium" 49 ms on a 184-node example; `decrossOpt` is a MILP "only for small graphs" ([d3-dag](https://erikbrinkman.github.io/d3-dag/)). Issue: 900 nodes / 1,600 edges took ~5 min with quad coordinates ([#68](https://github.com/erikbrinkman/d3-dag/issues/68)) |
| **yFiles** | "Calculating layouts for ten thousand nodes typically requires at least half a minute". Run full layouts rarely and incremental layouts often ([yFiles guide](https://docs.yfiles.com/yfiles-html/dguide/large_graph_performance/)) |
| **OGDF** (C++) | Sugiyama with pluggable phases, plus FastHierarchyLayout (Buchheim–Jünger–Leipert, ≤2 bends per edge). FMMM handles "hundreds of thousands of nodes" for **force** layout ([OGDF chapter](https://cs.brown.edu/people/rtamassi/gdhandbook/chapters/ogdf.pdf)) **[secondary]** |
| **Rust crates** | `rust-sugiyama` 0.4.0 (2025-09, ~40k downloads), `dagre` (Rust port) 0.1.1, `elkrs` 0.1.1 (claims a byte-exact ELK reimplementation; 2026-06), `mermaid-dagre` 0.1.5 (crates.io, 2026-10-05). All young; scale and quality unverified |
| **ForceAtlas2** (graphology, worker) | `graphology-layout-forceatlas2` 0.10.1 (2022; rc 0.11.0-rc1 2024-11, so slow-moving). Barnes-Hut is O(n log n). Ships a `FA2Layout` worker supervisor ([docs](https://graphology.github.io/standard-library/layout-forceatlas2.html)). FA2 ([Jacomy et al. 2014](https://journals.plos.org/plosone/article?id=10.1371%2Fjournal.pone.0098679)) is commonly cited as producing good layouts up to ~100,000 nodes **[secondary: search excerpt]** |
| **GPU force** | cosmos.gl: interactive in-browser layouts at 10^5–10^6 points. cuGraph FA2: 706,529 vertices / 1,238,568 edges in **4.8 s** (500 iterations, V100) vs 3 h 43 min in pure Python ([RAPIDS blog](https://medium.com/rapids-ai/large-graph-visualization-with-rapids-cugraph-590d07edce33)) **[secondary]**. Ogma's GPU layout: "tens of thousands of nodes within seconds", falling back to CPU above 10k ([changelog](https://doc.linkurious.com/ogma/latest/changelog.html)) **[vendor]** |

### Our layout benchmark **[measured here]**

Setup: Node 26 on one core. Synthetic "narrative-like" graph: layered generator, 1–5 out-choices per node (~3 avg), 85% of edges to the next layer and 15% skip 2–5 layers, 1% back edges (loops). Node box 120×40. Times include building the library's graph object. Timeouts are wall-clock.

| Engine / settings | 500 | 1k | 2k | 5k | 10k | 20k | 50k | 100k | 300k |
|---|---|---|---|---|---|---|---|---|---|
| dagre, network-simplex (default) | crash¹ | 51.8 s | — | — | — | — | — | — | — |
| dagre, longest-path | crash¹ | 3.9 s | crash¹ | crash¹ | — | — | — | — | — |
| dagre, longest-path, **acyclic input** | — | 0.84 s | 1.5 s | 4.4 s | crash¹ (~16 s) | crash¹ | crash¹ | — | — |
| elkjs layered, defaults | 1.3 s | 8.2 s | 10.4 s⁶ | 34.7 s⁶ | — | — | — | — | — |
| elkjs layered, "fast"² | 0.56 s | 1.3 s | 2.4 s | 10.4 s (1.9 GB heap) | >400 s | — | — | — | — |
| d3-dag, longestPath + twoLayer + simplex | 91 s | >120 s | — | — | — | — | — | — | — |
| d3-dag, longestPath + DFS + greedy ("fast") | — | 56 ms | 129 ms | 0.5 s | 2.7 s | 10.6 s | 132 s³ (1.07 GB) | **563 s** (1.9 GB) | not run⁵ |
| **Minimal linear pipeline⁴** (typed arrays) | — | — | — | — | **16 ms** | — | — | **73 ms** | **186 ms** (32 MB) |

- ¹ `Error: Not possible to find intersection inside of the rectangle`, a dagre failure on our cyclic multigraph (after ~18 s at 2k). Removing back edges fixes 2k–5k, but the same error returns at 10k+ even on acyclic input (probably parallel edges or overlapping geometry). **Cycles must be broken before handing a narrative graph to dagre, and dagre is not usable at 10k+ for this graph shape.**
- ² `thoroughness=1`, `LONGEST_PATH` layering, `SIMPLE` node placement, greedy switch off, `POLYLINE` routing.
- ³ Overlapped with another benchmark, so treat it as an upper bound.
- ⁴ DFS cycle breaking → longest-path layering (Kahn) → 4 barycenter sweeps without dummy nodes → grid x. **Quality is crude**: no dummy nodes, no Brandes–Köpf, very wide layers. It shows that the asymptotics are fine when you control the data structures. A production version needs dummy/long-edge handling, a better layering than longest-path, and Brandes–Köpf coordinates ([Brandes & Köpf 2001](https://link.springer.com/chapter/10.1007/3-540-45848-4_3)), which is still O(V+E) per pass.
- ⁵ Stopped: 100k already took 9.4 min and growth is ~quadratic (10k → 20k ×4, 50k → 100k ×4.3), so 300k would take well over an hour.
- ⁶ Acyclic input (back edges removed).

### Cited findings: incremental and stable layout
- ELK supports **interactive** strategies for cycle breaking, layering, crossing minimisation and node placement. The previous coordinates (`org.eclipse.elk.position`) drive the new ordering, and `crossingMinimization.semiInteractive` keeps in-layer order. `considerModelOrder` uses input order as a tie-breaker ([ELK layered overview 2025](https://eclipse.dev/elk/blog/posts/2025/25-08-21-layered.html); [semiInteractive option](https://eclipse.dev/elk/reference/options/org-eclipse-elk-layered-crossingMinimization-semiInteractive.html)).
- yFiles `HierarchicalLayout` offers "strong support for keeping an existing layout stable with respect to exact coordinates" for incremental insertion ([yFiles](https://docs.yfiles.com/yfiles-html/dguide/large_graph_performance/)).
- Online Sugiyama goes back to Graphviz's dynadag ([North & Woodhull, "On-line Hierarchical Graph Drawing", GD 2001](https://www.graphviz.org/documentation/NW01.pdf)). For process graphs, Mennens et al. compute **one global layout of the full graph** and show filtered subsets in place to keep views stable ([CGF 2019](https://onlinelibrary.wiley.com/doi/10.1111/cgf.13723)) **[unverified: publisher page returned 403; paraphrased from prior knowledge, title/authors confirmed via Crossref]**. This is exactly the pattern a spoiler-safe reader map needs (see Q5).
- Ashwell's taxonomy of choice-based structures (Time Cave, Gauntlet, **Branch and Bottleneck**, Quest, Open Map, Sorting Hat, Floating Modules, **Loop and Grow**, Spoke and Hub) ([Ashwell 2015](https://heterogenoustasks.wordpress.com/2015/01/26/standard-patterns-in-choice-based-games/)) suggests narrative-specific decompositions. **Bottleneck passages** are dominators of later content ([Lengauer & Tarjan 1979](https://doi.org/10.1145/357062.357071)), so the segments between them can collapse into "diamond" meta-nodes. **Loops** are strongly connected components and can be condensed into hub meta-nodes before layering.

---

## Q3. Exploration patterns for huge graphs

### Takeaway
The large-graph visualization literature (von Landesberger et al. 2011 survey) and every production system converge on the same pattern: **bound the number of drawn elements per view and let structure carry the overview.** Concretely:
1. **Overview first, zoom and filter, details on demand** ([Shneiderman 1996](https://doi.org/10.1109/VL.1996.545307)). Here the "overview" is a **meta-graph** of authored units (volume → chapter → scene), not 300k dots.
2. **Aggregation into meta-nodes**, using authored hierarchy where available and otherwise algorithmic clustering, with **expand-on-demand** (ASK-GraphView: hierarchy over graphs up to **16M edges**, navigated top-down by expanding clusters ([Abello, van Ham, Krishnan 2006](https://doi.org/10.1109/TVCG.2006.120))).
3. **Semantic zoom / LOD via a tile pyramid** (GraphMaps). Each zoom level shows a refinement, drawn entities are capped per view, labels go to the highest-ranked nodes, and geometry is stable across zooms ([Nachmanson et al. 2015](https://arxiv.org/abs/1506.06745)). A 2026 browser implementation reaches 95% of frame gaps ≤18.4 ms with `TileCapacity`=500 and budgets ~200 bytes per element. It builds the whole pyramid up front: 14–23 s tiling for 28k–32k-node graphs, 173 s end-to-end including layout. The authors list lazy construction as future work ([Nachmanson & Chen 2026, arXiv 2605.17498](https://arxiv.org/html/2605.17498)).
4. **Focus + context / degree-of-interest**: "Search, Show Context, Expand on Demand" adapts Furnas' DOI from trees to graphs to extract a relevant subgraph around a focus node ([van Ham & Perer 2009](https://doi.org/10.1109/TVCG.2009.108)). For authors, this is "show me everything within k choices of this passage, plus the paths to the nearest bottleneck and ending".
5. **Edge bundling** reduces ink: hierarchical edge bundling ([Holten 2006](https://doi.org/10.1109/TVCG.2006.147)), force-directed edge bundling ([Holten & van Wijk 2009](https://doi.org/10.1111/j.1467-8659.2009.01450.x)), and bundling specifically **for Sugiyama layouts** by minimising ink ([Pupyrev, Nachmanson, Kaufmann, GD 2010](https://link.springer.com/chapter/10.1007/978-3-642-18469-7_30)). With meta-nodes, the cheapest "bundle" is a **single aggregated edge per (cluster A → cluster B) with a count**.
6. **Interaction-time simplification**: hide edges and labels while moving (sigma `hideEdgesOnMove`, G6 `optimize-viewport-transform` above ~500 elements), and switch to cheaper renderers below a zoom threshold. yFiles' large-graph demo uses WebGL at low zoom and SVG at high zoom ([yFiles demo](https://www.yfiles.com/demos/showcase/large-graphs/)).
7. **Minimap and "jump to"**: Twine added "Go To" (2.6). Yarn Spinner has "Jump to Node". Both are cheap and expected.

### Cited findings
- von Landesberger et al., "Visual Analysis of Large Graphs: State-of-the-Art and Future Research Challenges", *Computer Graphics Forum* 30(6):1719–1749, 2011 ([Wiley](https://onlinelibrary.wiley.com/doi/abs/10.1111/j.1467-8659.2011.01898.x)). This is the standard survey of aggregation, filtering, interaction and algorithmic analysis for large graphs.
- GraphMaps renders "only those entities of the layer that intersect the current viewport", so "the number of entities rendered at each view does not exceed a predefined threshold" ([arXiv 1506.06745](https://arxiv.org/abs/1506.06745)).
- Ogma: "Text is really expensive in term of graphics memory. One character uses as much space as a node shape" ([Ogma best practices](https://doc.linkurious.com/ogma/latest/tutorials/best-practices/)) **[vendor]**. **Label LOD matters as much as node LOD.**
- Dagster at 10k+ assets: viewport virtualisation ("only draw an edge if its to/from node is visible"), per-asset data fetching, an IndexedDB layout cache keyed by a SHA-1 of the graph, and collapsible groups ([Dagster](https://dagster.io/blog/scaling-dag-visualization)).

---

## Q4. How narrative tools present large story maps

### Takeaway
Shipping narrative tools almost never show a flat map of everything. They nest: **articy** (flow fragments and dialogues you "submerge" into), **Yarn Spinner** (file containers and `cluster` groups), **Arcweave** (boards and jumpers). The flat-map tool, **Twine**, degrades at ~500–800 passages, and its power users move to text (Twee/Tweego) or split stories across files. ink deliberately has no map. Visualizations of whole branching works come from analysts, not editors (Swinehart's CYOA study).

### Cited findings
- **articy:draft X**: "The flow supports nesting, which means that you can submerge in nodes and view their inner structure", with emerge via Ctrl+Backspace ([articy help](https://www.articy.com/help/adx/UI_View_Flow.html)). articy notes that a single large flow layer becomes cumbersome and "might even be noticeable in the performance of the application". It recommends Flow Fragments as containers, organized "from macro to micro" ([articy nesting tutorial](https://www.articy.com/en/improve-project-structure-with-nesting/)). The Flow view also has "Expand selection" to all nodes reachable from an output pin or leading to an input pin, i.e. built-in reachability highlighting ([help](https://www.articy.com/help/adx/UI_View_Flow.html)).
- **Disco Elysium** (articy, >1M words): lead writer Helen Hindpere said articy at one point froze because "it definitely wasn't built for it" ([PC Gamer](https://www.pcgamer.com/games/rpg/disco-elysium-had-so-much-text-it-broke-the-branching-narrative-software-we-were-writing-too-much/); [DualShockers](https://www.dualshockers.com/disco-elysium-script-length-one-million-words/)) **[secondary: search excerpt]**.
- **Twine 2** (2.10.0, Nov 2024): lag "around 700 or so passages", driven mostly by "how many connection arrows need to be drawn rather than the number of passages". Workarounds: "Show only story structure" view, split by chapter, or VS Code + Twee/Tweego ([intfiction forum](https://intfiction.org/t/performance-with-so-many-passages/48097); [twinejs#766](https://github.com/klembot/twinejs/issues/766), lag at "more than 600-800 passages") **[community]**. 2.6 added a "Go To" passage button ([release notes](https://twinery.org/reference/en/release-notes/2-6.html)). We found no later story-map performance rewrite in the 2.7–2.10 notes.
- **Yarn Spinner VS Code Graph View**: File view vs **Project view**, where "Each `.yarn` file is shown as a container with its nodes inside". Cross-file jumps appear as stub nodes and as "dashed coloured lines between file containers". A `cluster` header groups nodes visually. Positions are saved in each node's `position` header. It also has auto-layout and "Jump to Node" ([Yarn Spinner docs](https://docs.yarnspinner.dev/write-yarn-scripts/yarn-spinner-editor/writing-yarn-in-vs-code)). This is a good minimal model: **container = file/chapter, stub = cross-container edge**.
- **Arcweave**: boards are "canvases where you design your story flowcharts". **Jumpers** are "links to or aliases of elements", used to connect distant nodes without long wires ([Arcweave docs](https://docs.arcweave.com/project-items/overview)).
- **Inky / ink**: no flowchart by design. The manifesto line is "ditch the flow-charts! If you want to see how your story is structured, then play it!" Third-party tools such as inkgraph render knots and diverts ([Introduction to Ink, Ingold](https://medium.com/game-writing-guide/introduction-to-ink-3e6c224865f8); [inkgraph](https://github.com/taugit/inkgraph)) **[secondary]**.
- **Ren'Py**: no built-in map. Community tools: renpy-graphviz (labels, jumps and calls via Graphviz, with tags such as BREAK/IGNORE/SKIPLINK to prune) ([repo](https://github.com/EwenQuim/renpy-graphviz)) and the "Ren'Py Flow" VS Code extension. In-game route maps are hand-built on `renpy.seen_label()` ([Ren'Py docs](https://www.renpy.org/doc/html/label.html)).
- **Larian (BG3)**: the released toolkit includes a node-based Dialogue Editor with per-node flag properties and nested dialogs ([BG3 modding docs](https://docs.baldursgate3.game/index.php?title=Journal%3A_Using_Quest_Flags_in_Dialogue)). Swen Vincke: "We are a game development company, we're not a tools company" ([PC Gamer](https://www.pcgamer.com/games/rpg/baldurs-gate-3-mod-support-GDC-update/)). We found no public talk on Larian's large-scale story-map tooling.
- **Christian Swinehart, "One Book, Many Readings"** (2009; updated 2020, 2022): CYOA books drawn as page grids colour-coded by type (decision, good or bad ending, linear), **arc diagrams** whose thickness is "how commonly each transition appears across all possible readings", and force-directed views. It shows endings declining across the series ([samizdat.co/cyoa](https://samizdat.co/cyoa/)). Weighting edges by path frequency is a strong encoding for authors.
- **80 Days** (inkle): ~700,000 words, 150 cities, 16,000 choices; one playthrough sees under 10%. The **globe is the reader-facing map**: Ingold argues maps "proved" branching by making visible what you missed ([IF50: 80 Days](https://if50.substack.com/p/2014-80-days)). Lesson: a **semantic** map (geography, timeline) can replace a node-link map for readers.

---

## Q5. Reader-facing "path map" UX in games

### Takeaway
The common design is **per-chapter flowcharts plus a chapter-level overview**, with **progressive reveal**:
- visited nodes drawn fully;
- branch points show that alternatives exist (locked or masked boxes, "???");
- authored **locks** for cross-route dependencies;
- **jump-to** any visited node, i.e. resume from that point.

Optional **community percentages** add a social layer. Fog-of-war is not decoration: these maps are bounded by what the reader has seen, so they stay small (hundreds to low thousands of nodes) even for huge works.

### Cited findings
| Game | Structure | Unseen content | Extras |
|---|---|---|---|
| **Detroit: Become Human** (Quantic Dream) | Per-chapter flowchart, shown after each chapter and from the menu | Unmade choices are masked and locked until the matching choice is made; paths cut off by earlier actions are greyed out ([Medium review](https://medium.com/@chiawei.liu/detroit-become-human-review-cd68cbffc3e6); [100% guide](https://medium.com/@epicshane/getting-100-completion-of-detroit-become-human-bc2f2ba8e462)) **[secondary: search excerpts]** | Global % of players per node ([Interactive Pasts](https://interactivepasts.com/how-games-tell-tales-part-3-detroit-become-human-freedom-of-choice-and-intended-play/)) |
| **As Dusk Falls** (Interior/Night) | Per-chapter "story tree" after each chapter; "Explore Story Tree" on replay | Shows how many outcomes or branches exist and roughly where new routes unlock, without saying what they are ([TrueAchievements](https://www.trueachievements.com/game/As-Dusk-Falls/walkthrough/12)) **[secondary]** | Community % |
| **Zero Escape: VLR / ZTD** (Spike Chunsoft) | VLR: one FLOW chart of all routes with escape rooms, novel sections and locks. ZTD: a global flowchart shared by three teams plus a small flowchart per "fragment" ([GameFAQs](https://gamefaqs.gamespot.com/boards/175993-zero-escape-zero-time-dilemma/73946743)) **[community]** | Black lock = blocked until another route supplies information; cyan = open ([GameFAQs VLR](https://gamefaqs.gamespot.com/boards/641335-zero-escape-virtues-last-reward/68638979)) **[community]** | Jump to any seen point |
| **AI: The Somnium Files** | Flowchart by route (red/blue/green/yellow/pink) and day; every Somnium has its own entry | Gated routes; a lock is placed and later animated open ([Twinfinite](https://twinfinite.net/guides/ai-somnium-files-how-to-use-the-flowchart/)) **[secondary]** | Replay any node |
| **YU-NO** (A.D.M.S.) | Branching tree of parallel worlds | Shows **only discovered** branches; undiscovered ones appear only when encountered. Marks current location, jewel saves, endings and "???" points ([Spike Chunsoft system page](http://spike-chunsoft.com/YU-NO/system/)) | Jewels = save markers placed on the map. Items carry across branches |
| **428: Shibuya Scramble** | **Time chart**: each character's tale in 5-minute blocks; jump to any character once a time threshold is reached ([RPGFan](https://www.rpgfan.com/review/428-shibuya-scramble/)) | "Keep Out" barriers resolved by zapping from another character's scene | TIPS hyperlinks |
| **Steins;Gate / Elite** | **No built-in flowchart**; route maps are fan-made ([Steam discussion](https://steamcommunity.com/app/819030/discussions/0/1840188800794044811)) **[community]** | — | — |
| **13 Sentinels: Aegis Rim** | Event Archive browsable by sector, loop and character, filled in as Remembrance mode progresses ([wiki](https://13-sentinels-aegis-rim.fandom.com/wiki/Event_Archive)) **[community]** | Unvisited events absent | Multiple facets over one event set |
| **Telltale / Until Dawn** | Not graphs: end-of-episode choice % ([Walking Dead wiki](https://walkingdead.fandom.com/wiki/Telltale_Series_Statistics)); Until Dawn's 22 "Butterfly Effect" chains fill in as choices are made ([wiki](https://until-dawn.fandom.com/wiki/Butterfly_Effect)) **[community]** | — | Shows the consequence chain without topology |
| **Slay the Spire** (non-narrative analogue) | Per-act layered DAG map (7×15 grid; 1–3 in/out paths per room; paths never cross) ([wiki](https://slaythespire.wiki.gg/wiki/Map_Generation)) **[community]** | Topology and node **types** visible, content hidden | Shows that structure can be revealed without spoiling content |

Design implications for Narrata readers:
1. **The unit is the chapter.** Show a chapter overview (meta-graph of chapters the reader has entered) and a per-chapter flowchart.
2. **Three reveal levels**, chosen per work by the author: (a) *visited only* (YU-NO); (b) *visited + frontier stubs* (unchosen choices at visited nodes as anonymous "?" boxes; the reader already saw the choice text, so it can be shown); (c) *structure without content* (Detroit/Slay the Spire style, author opt-in).
3. **Authored locks and gates** belong in the map, because readers need to know a path is blocked rather than missing.
4. **Jump-to = restore a saved session** at that node. This maps onto Narrata's isolated/persistent branching sessions: each map node can link to the reader's own snapshot.
5. **Community % only with aggregation thresholds** (k-anonymity), and only for nodes the reader has visited.

---

## Q6. Data delivery and memory

### Takeaway
Ship **columnar binary chunks per cluster** (chapter/tile) with positions precomputed, decode in a Web Worker, and transfer `ArrayBuffer`s to the renderer. In our test, JSON was ~3× larger raw and ~1.8× larger compressed than columnar binary. The bigger cost is materializing per-node and per-edge JS objects: ~35× the memory of typed arrays (393 MB vs 11 MB at 300k/900k). **Arrow IPC** is a good interchange format if the stack already uses Arrow or DuckDB (Cosmograph does), but apache-arrow JS costs **52 KB gz**. A **minimal custom typed-array container** (header + `Float32Array` x/y + `Uint32Array` ids/cluster + CSR edge arrays) needs ~0 KB of library and decodes zero-copy. FlatBuffers (3 KB gz) suits small structured metadata, not bulk columns.

### Measurements **[measured here]** (synthetic graph; real compression depends on content; labels excluded)

| Representation | 100k / 300k | 300k / 900k |
|---|---|---|
| JSON (`{id,x,y,c}` + `[src,dst]` id strings) raw / gz / br | 10.1 / 1.55 / 1.43 MB | 32.1 / 4.67 / 4.28 MB |
| Arrow IPC nodes (x,y f32 + cluster u32) raw / gz / br | 1.2 / 0.23 / 0.16 MB | 3.6 / 0.69 / 0.46 MB |
| Arrow IPC edges (u32 src,dst) raw / gz / br | 2.4 / 0.77 / 0.63 MB | 7.2 / 2.32 / 1.88 MB |
| Edges sorted + delta + zigzag varint raw / gz / br | 0.89 / 0.52 / 0.49 MB | 2.68 / 1.55 / 1.42 MB |
| In-memory typed arrays (x,y,cluster,flags,src,dst) | 3.7 MB | 11.1 MB |
| In-memory graphology `MultiDirectedGraph` (5 node attrs, no edge attrs), V8 heap | 135 MB (~337 B/element) | 393 MB (~327 B/element) |
| Browser JS-heap Δ, sigma v3 + graphology | +150 MB | +456 MB |
| Browser JS-heap Δ, cosmos.gl (typed arrays) | +83 MB | +220 MB |

Other references: GraphMaps-in-browser budgets **~200 B per element** ([arXiv 2605.17498](https://arxiv.org/html/2605.17498)). Graphistry: 10M edges in 537 MB (~54 B per edge, all-in) ([Graphistry](https://www.graphistry.com/blog/graphistry-2-53-0-large-graph-visualization-at-10-million-edges)) **[vendor]**. Arrow's IPC format is designed so fixed-width buffers can be read without copying ([Arrow columnar spec](https://arrow.apache.org/docs/format/Columnar.html)). The underlying `ArrayBuffer`s can be posted to and from workers as Transferables.

### Platform constraints (Cloudflare)
- **Workers**: 128 MB memory per isolate (JS heap + WASM); CPU up to 5 min per request on paid plans (default 30 s); Queue consumers default to 30 s CPU, with 15-min wall time for cron and queues ([Workers limits](https://developers.cloudflare.com/workers/platform/limits/)). **A 300k/900k graph does not fit an object-model layout job inside a Worker** (graphology alone ≈390 MB). Even typed-array layouts need care to stay under 128 MB.
- **Containers**: custom instance types up to **4 vCPU / 12 GiB** ([changelog 2026-01-05](https://developers.cloudflare.com/changelog/post/2026-01-05-custom-instance-types/)). This is where server-side layout and tiling of a whole work belongs, if it is not done offline in the CLI at pack time.
- No response-size limit in Workers. CDN cache object limit is 512 MB (non-Enterprise) ([limits](https://developers.cloudflare.com/workers/platform/limits/)). Precomputed chunks fit R2 + cache, with content-hash URLs for immutability.

---

## Recommended architecture

### What must be precomputed (offline at pack/publish time in the Rust CLI, or in a Cloudflare Container job; never in a Worker request)
1. **Normalized topology**: condense parallel choice edges (keep the choice-id list and count); find SCCs and back edges (loops) and tag them; compute reachability from the start, unreachable or orphan nodes, dead ends, ending sets, and dominators / bottleneck passages.
2. **Hierarchy**: authored units (volume → chapter → scene → passage) first. Where a unit is too big (cap ~2–5k nodes) or missing, derive sub-clusters from bottleneck segments and SCC condensation, then community detection as a fallback.
3. **Layout per hierarchy level**, deterministic and **per cluster**: layered layout inside each scene or chapter, then pack the chapters in the meta-graph's layered layout. Store `(chapter, layer, orderKey)` plus final `x,y`. Order keys should be fractional/lexicographic so that later edits insert without renumbering. Our measurement says a typed-array pipeline handles the whole work in well under a second, and per-chapter jobs parallelize. Write it once in Rust, so the same code runs in the CLI, a Container, and a WASM worker for local incremental relayout.
4. **LOD / tile pyramid**: level 0 = chapter meta-graph with aggregated edges carrying counts; level 1 = scenes; level 2 = passages in spatial tiles (cap ~500–2,000 elements per tile). Include per-node importance (bottleneck, degree, path-frequency weight à la Swinehart, endings) to decide which **labels** appear at each zoom.
5. **Search index** (title/text → node id + tile + coordinates) and **analysis overlays** (reachability, depth, endings reachable, path counts, word counts) as per-node columns.
6. **Reader-side projections**: no per-reader data is precomputed. Reader maps are produced on request from the reader's visit log and the precomputed layout (see below).

### Author view
- **Renderer**: sigma.js v3 + graphology (MIT, ~37 KB gz) in a lazy route chunk. Plan the v4 upgrade (WebGL labels with picking; stable expected ~Oct 2026). Use `nodeReducer`/`edgeReducer` for highlight and filter overlays, `refresh({partialGraph})` for edits, `hideEdgesOnMove` on weak GPUs, and `labelRenderedSizeThreshold`/`labelDensity` for label LOD.
- **Working-set budget**: keep ≤ ~50k nodes / ~150k edges in graphology at once (60 fps on Intel UHD with short edges; ~75–150 MB heap). Move between levels by semantic zoom and by "submerge into chapter" (articy-style). Load and evict chapter tiles as needed. Never instantiate the whole 300k graph as JS objects on the client.
- **Whole-work overview**: the level-0 meta-graph, plus a density or heat overlay. If a literal "every passage" galaxy is wanted for analysis, lazy-load **cosmos.gl** (MIT, ~189 KB gz) with `enableSimulation:false` and the precomputed positions (300k/900k at 40 fps on an iGPU, +220 MB). Avoid **Cosmograph** (CC BY-NC).
- **Rich local editing**: open a focused subgraph (k-hop DOI around the selected passage, or one scene ≤ ~500 nodes) in **React Flow** with full DOM node cards. Keep the global map in WebGL.
- **Incremental layout on edit**: place new or changed nodes using the stored order keys (ELK-style semi-interactive), relayout only the affected scene in a worker, and re-run the chapter layout in the background or on save. Positions of untouched chapters never move.
- **Exploration tools**: jump-to/search; reachability "expand selection" (articy); k-hop focus+context; aggregated edge bundles between clusters; minimap of the level-0 graph; loop and SCC badges instead of long back edges.

### Reader view
- **Server builds the map** from the reader's visited node and edge set (from session or branch history). It returns only visited nodes, visited edges and (optionally) frontier stubs, with **opaque per-response ids**. Never ship the full graph and hide parts in CSS: anything sent is a spoiler.
- **Layout = projection of the global layout, compacted**: sort revealed nodes by the precomputed `(chapter, layer, orderKey)`, then assign compacted ranks so that gaps do not leak the size of hidden branches. New reveals insert without reordering existing nodes, which keeps the map stable (Mennens-style "one global layout, filtered views").
- **Per-chapter flowchart + chapter overview** (Detroit, As Dusk Falls, Zero Escape). Locks and gates come from authored metadata. Jump-to restores the reader's snapshot. Community % appears only above aggregation thresholds.
- **Size**: bounded by exploration, typically 10^2–10^3 nodes (80 Days: under 10% per playthrough). Canvas or sigma suffice, or even SVG for a single chapter. Reuse the author renderer chunk for consistency, or a tiny custom canvas renderer to keep the reader page budget minimal.

### Library decision summary
| Need | Choice | Why not the others |
|---|---|---|
| Author map (interactive, labels, editing, ≤50k drawn) | **sigma.js v3→v4 + graphology** | G6 (433 KB gz) and Cytoscape (140 KB gz, WebGL still preview, canvas ~3–5k) are heavier or slower. React Flow is DOM-bound (~1k). vis-network is canvas/physics-first. Ogma and yFiles are commercial |
| Whole-work galaxy (≤1M points, analysis) | **cosmos.gl** (lazy) or precomputed tiles | Cosmograph is CC BY-NC. deck.gl graph-layers is immature |
| Small rich subgraph editor | **React Flow** | Canvas libraries are worse at form-like node UIs |
| Layered layout | **Own Rust pipeline** (CLI/Container/WASM), per cluster; elkjs only for small interactive subgraphs in a worker | dagre, elkjs and d3-dag measured super-linear (dagre crashes on cycles). Graphviz dot takes hours at 350k |
| Wire format | **Typed-array chunks** (Arrow IPC if DuckDB/Arrow is adopted elsewhere) | JSON is ~3× bigger raw (~1.8× compressed) and forces per-element object churn |

---

## Open questions / risks
- **Quality of a custom layered layout** (crossings, long-edge routing) versus ELK. Validate on real Narrata works and consider ELK (Java, in a Container) as an offline quality baseline.
- **sigma v4 stability and migration cost** (renderer API rewrite). Pin v3 until v4 ships stable.
- **Mobile GPUs**: cosmos.gl needs float-texture extensions (iOS/Android caveats). sigma at 30k on a phone GPU is unmeasured.
- **Spoiler leakage channels**: node counts per chapter, layer gaps, edge stubs and timing. Decide per work what the reader map may reveal.
- **Path counts explode combinatorially** in branch-and-merge graphs. Store log-scale or saturated counts.

---

## Appendix A. Benchmark method and caveats **[measured here]**
- Machine: Windows 11 laptop. dGPU NVIDIA RTX 4080 Laptop (12 GB); iGPU Intel UHD Graphics, selected with Chrome's `--force_low_power_gpu`. Chrome stable via puppeteer-core 25.12.0, headful, vsync on (60 Hz cap), `--enable-precise-memory-info`, `--js-flags=--expose-gc`. Node v26.5.0.
- Graph generator: layered synthetic, ~3 out-edges per node, 85% to the next layer, 15% skip 2–5 layers, 1% back edges. Variant "random long": targets uniformly random within the target layer, giving long crossing edges (hairball). Variant "short, ordered": targets near the source's relative x (±8), mimicking a well-ordered layered layout. Synthetic graphs have many sources (no single start) and uniform degree, so real works will differ.
- Heap deltas are `performance.memory.usedJSHeapSize` after GC, relative to the pre-build baseline (generator arrays excluded). GPU memory was not measured.
- FPS is the mean `requestAnimationFrame` rate while the camera is updated every frame (sigma `camera.setState`, cosmos `setZoomLevel(…, 0)`). p95 is the 95th-percentile frame interval.
- Bundle sizes: esbuild 0.28.2, `--bundle --minify --format=esm`, gzip -9 / brotli q11 via Node zlib. Entries: `sigma`+`graphology`; `@cosmos.gl/graph` `Graph`; `cytoscape`; `@antv/g6` `Graph`; `@xyflow/react` `ReactFlow, MiniMap, Controls` (React external); `vis-network/standalone` `Network`; `elkjs/lib/elk.bundled.js`; `@dagrejs/dagre`; `d3-dag` `sugiyama`; `apache-arrow` `tableFromIPC`; `flatbuffers`; `@deck.gl/core`+`@deck.gl/layers` (Scatterplot/Line/Text); `@viz-js/viz`; `@hpcc-js/wasm-graphviz`; `react`+`react-dom/client` = 67 KB gz for reference.
- Layout benchmarks ran single-threaded in Node with wall-clock timeouts. Some runs overlapped with browser benchmarks (flagged).
