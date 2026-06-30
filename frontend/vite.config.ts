// Vite configuration tuned for Windows 7 compatibility. The build target is
// es2017 so the output runs on Chrome 109 and Firefox ESR 115 without relying
// on a modern polyfill runtime. The dev server proxies /api and /metrics to
// the Rust backend during local development.
import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';

export default defineConfig({
  plugins: [react()],
  build: {
    target: 'es2018',
    outDir: '../backend/static',
    emptyOutDir: true,
    sourcemap: false,
    chunkSizeWarningLimit: 1024,
  },
  server: {
    port: 5173,
    proxy: {
      '/api': 'http://127.0.0.1:8080',
      '/metrics': 'http://127.0.0.1:8080',
    },
  },
});
