import { defineConfig } from "vitest/config";
import react from "@vitejs/plugin-react";

export default defineConfig({
  base: "./",
  publicDir: false,
  plugins: [react()],
  build: {
    sourcemap: true,
    target: "es2022",
    chunkSizeWarningLimit: 1024,
  },
  test: {
    environment: "jsdom",
    globals: true,
    setupFiles: "./src/test/setup.ts",
    include: ["src/**/*.{test,spec}.{ts,tsx}"],
    css: true,
    coverage: {
      reporter: ["text", "html"],
    },
  },
});
