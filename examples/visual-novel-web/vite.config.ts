import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";

const here = fileURLToPath(new URL(".", import.meta.url));
const product = resolve(here, "../../products/visual-novel-demo");
const packageRoot = resolve(here, "../../packages/narrata/web");

export default defineConfig({
  plugins: [react()],
  publicDir: resolve(product, "public"),
  resolve: { dedupe: ["react", "react-dom"] },
  server: {
    // The workspace file dependency resolves here; installed consumers remove this allowance.
    fs: { allow: [here, product, packageRoot] },
  },
  build: { outDir: "dist", emptyOutDir: true },
});
