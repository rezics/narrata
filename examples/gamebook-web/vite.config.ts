import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import { fileURLToPath } from "node:url";

export default defineConfig({
  plugins: [react()], worker: { format: "es" },
  // npm's local file dependency is a symlink. Wasm asset URLs resolve to the package's
  // actual directory; permit that dependency alongside the reader during development.
  server: { fs: { allow: [
    fileURLToPath(new URL(".", import.meta.url)),
    fileURLToPath(new URL("../../packages/narrata/web", import.meta.url)),
  ] } },
});
