import { defineConfig } from 'vite';

export default defineConfig({
  // top-level await (used to load the WASM module once)
  build: { target: 'es2022' },
  optimizeDeps: { exclude: ['geoprecise'], esbuildOptions: { target: 'es2022' } },
});
