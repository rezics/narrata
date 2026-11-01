import { fileURLToPath } from "node:url";
import { join, resolve } from "node:path";
import { loadVnAuthoring } from "../../../packages/narrata/web/dist/vn/server.js";
import { publishReferenceVn } from "./publish-lib.mjs";

const root = fileURLToPath(new URL("../../..", import.meta.url));
const product = resolve(root, "products/visual-novel-demo");
const result = await publishReferenceVn({
  authoring: await loadVnAuthoring(),
  sourceDirectory: product,
  packPath: join(product, "story.narpack"),
  publicStoryDirectory: join(product, "public/story"),
  generatedPublicationPath: join(root, "examples/visual-novel-web/src/generated/publication.ts"),
});
console.log(JSON.stringify({ ...result, pack: undefined }, null, 2));
