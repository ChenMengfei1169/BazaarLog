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
    // Pin the dev server's CORS allow-list explicitly instead of relying on the
    // bundler default. A permissive dev server lets any website the developer
    // visits read responses from http://localhost:5173 (GHSA-67mh-4wv8-2f99).
    cors: {
      origin: [/^https?:\/\/(?:(?:[^:]+\.)?localhost|127\.0\.0\.1|\[::1\])(?::\d+)?$/],
    },
    proxy: {
      '/api': 'http://127.0.0.1:3000',
      '/metrics': 'http://127.0.0.1:3000',
    },
  },
});