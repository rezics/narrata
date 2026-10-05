import { createGraphController, mountAuthorGraph, type GraphOptions } from "@rezics/narrata/graph";
import { AuthorGraph } from "@rezics/narrata/graph/react";
const options: GraphOptions = { indexId: "0".repeat(64), fetchObject: async () => new Uint8Array(), resolver: { resolve: async () => [] } };
const controller = createGraphController(options);
void controller.enterCluster(0);
void controller.enterPartition(0, 0);
void (<AuthorGraph options={options} onController={value => { value?.dispose(); }} />);
if (typeof document !== "undefined") mountAuthorGraph(document.createElement("div"), options).dispose();
