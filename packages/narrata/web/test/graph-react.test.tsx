import { expect, test } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";
import { AuthorGraph } from "@rezics/narrata/graph/react";
import type { GraphOptions } from "@rezics/narrata/graph";

test("React graph is SSR safe and gives hosts an accessible, sized mount point", () => {
  const options: GraphOptions = { indexId: "0".repeat(64), fetchObject: async () => { throw new Error("SSR must not fetch"); }, resolver: { resolve: async () => [] } };
  const html = renderToStaticMarkup(<AuthorGraph options={options} label="Author structure" style={{ height: 480 }} />);
  expect(html).toContain('role="img"');
  expect(html).toContain('aria-label="Author structure"');
  expect(html).toContain("height:480px");
});
