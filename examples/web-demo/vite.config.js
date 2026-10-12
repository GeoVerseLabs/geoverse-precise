import { defineConfig } from 'vite';

export default defineConfig({
  // 部署到 GitHub Pages 时为 /geoverse-precise/，本地开发保持 /
  base: process.env.DEMO_BASE || '/',
  // top-level await (used to load the WASM module once)
  build: { target: 'es2022' },
  optimizeDeps: { exclude: ['@geoverse/geoverse-precise'], esbuildOptions: { target: 'es2022' } },
});
