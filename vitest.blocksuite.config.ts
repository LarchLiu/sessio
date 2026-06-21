import { defineConfig } from 'vitest/config';

export default defineConfig({
  test: {
    projects: ['./blocksuite/**/*/vitest.config.ts'],
  },
});
