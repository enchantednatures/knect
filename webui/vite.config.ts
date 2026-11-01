import { svelte } from '@sveltejs/vite-plugin-svelte';
import tailwindcss from '@tailwindcss/vite';
import { defineConfig } from 'vitest/config';

export default defineConfig({
  plugins: [svelte(), tailwindcss()],
  base: '/',
  resolve: {
    // Vitest defaults to the node condition; Svelte 5's client runtime and
    // component tests need the browser build.
    conditions: ['browser', 'module', 'import', 'default'],
  },
  build: {
    outDir: 'dist',
    assetsDir: 'assets',
    sourcemap: false,
    target: 'es2022',
    cssCodeSplit: true,
    reportCompressedSize: true,
    rollupOptions: {
      output: {
        manualChunks(id) {
          if (id.includes('node_modules/@xyflow') || id.includes('node_modules/dagre')) {
            return 'graph';
          }
          if (id.includes('node_modules/svelte')) {
            return 'svelte';
          }
          return undefined;
        },
      },
    },
  },
  server: {
    port: 5173,
    proxy: {
      '/rpc': 'http://127.0.0.1:7420',
      '/events': 'http://127.0.0.1:7420',
      '/healthz': 'http://127.0.0.1:7420',
    },
  },
  test: {
    environment: 'jsdom',
    include: ['src/tests/**/*.test.ts'],
    setupFiles: ['./src/tests/setup.ts'],
    restoreMocks: true,
    clearMocks: true,
    css: false,
  },
});
