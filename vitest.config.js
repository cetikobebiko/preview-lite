import { defineConfig } from 'vitest/config'
import { svelte } from '@sveltejs/vite-plugin-svelte'
import { fileURLToPath } from 'node:url'

export default defineConfig({
  plugins: [svelte()],
  test: {
    environment: 'jsdom',
    globals: true,
    // B5 fail-closed hardening (§18):
    // - no test files matched => FAIL (not silent PASS)
    // - .only without --allowOnly => FAIL (prevents committed .only green)
    // - aggregate must have at least one test
    passWithNoTests: false,
    allowOnly: false,
  },
  resolve: {
    conditions: ['browser'],
    alias: {
      $lib: fileURLToPath(new URL('./src/lib', import.meta.url)),
    },
  },
})
