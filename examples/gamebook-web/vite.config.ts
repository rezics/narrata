import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import { fileURLToPath } from "node:url";
import { createReferenceHost } from "./scripts/reference-host.ts";

export default defineConfig({
  plugins: [react(), {
    name: "public-authored-reference-host",
    async configureServer(server) {
      const host = await createReferenceHost();
      server.httpServer?.once("close", host.close);
      server.middlewares.use((request, response, next) => {
        void host.handle(request, response).then(handled => { if (!handled) next(); }).catch(next);
      });
    },
  }], worker: { format: "es" },
  // The linked package has its own development React. Hooks and React DOM must share
  // the host's instance in production builds as well as the dependency-optimized dev server.
  resolve: { dedupe: ["react", "react-dom"] },
  // npm's local file dependency is a symlink. Wasm asset URLs resolve to the package's
  // actual directory; permit that dependency alongside the reader during development.
  server: { fs: { allow: [
    fileURLToPath(new URL(".", import.meta.url)),
    fileURLToPath(new URL("../../packages/narrata/web", import.meta.url)),
    fileURLToPath(new URL("../../products/gamebook-demo", import.meta.url)),
  ] } },
});
