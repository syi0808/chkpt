import { defineConfig } from 'vitest/config'

export default defineConfig({
  test: {
    testTimeout: 30000,
    include: ['__test__/**/*.spec.ts'],
  },
})
