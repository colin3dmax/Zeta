import { defineConfig } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";
import { resolve } from "node:path";

export default defineConfig({
  plugins: [svelte()],
  build: {
    outDir: "dist",
    emptyOutDir: true,
    rollupOptions: {
      input: {
        index: resolve(__dirname, "index.html"),
        "playground-component": resolve(__dirname, "src/playground-component.js"),
      },
      output: {
        entryFileNames: "assets/[name].js",
      },
    },
  }
});
