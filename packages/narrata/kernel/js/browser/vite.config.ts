import { fileURLToPath } from "node:url";

import { defineConfig } from "vite";

const browser = fileURLToPath(new URL(".", import.meta.url));
const pkg = fileURLToPath(new URL("..", import.meta.url));

export default defineConfig({ root: browser, server: { fs: { allow: [pkg] } } });
