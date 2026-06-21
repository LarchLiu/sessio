import { playwright } from '@vitest/browser-playwright';
import { defineConfig } from 'vitest/config';

import { defineBlocksuiteBrowserProject } from '../vitest.shared';

const browserInstances =
  process.env.CI === 'true' || process.env.BLOCKSUITE_ALL_BROWSERS === 'true'
    ? [
        { browser: 'chromium' as const },
        { browser: 'firefox' as const },
        { browser: 'webkit' as const },
      ]
    : [{ browser: 'webkit' as const }];

export default defineConfig(_configEnv =>
  defineConfig(defineBlocksuiteBrowserProject({
    esbuild: { target: 'es2018' },
    optimizeDeps: {
      force: true,
      esbuildOptions: {
        // Vitest hardcodes the esbuild target to es2020,
        // override it to es2022 for top level await.
        target: 'es2022',
      },
    },
    test: {
      include: ['src/__tests__/**/*.spec.ts'],
      fileParallelism: false,
      retry: process.env.CI === 'true' ? 3 : 0,
      browser: {
        enabled: true,
        headless: true,
        instances: browserInstances,
        provider: playwright(),
        isolate: false,
        viewport: {
          width: 1024,
          height: 768,
        },
      },
      coverage: {
        provider: 'istanbul',
        reporter: ['lcov'],
        reportsDirectory: '../../.coverage/integration-test',
      },
      deps: {
        interopDefault: true,
      },
    },
  }))
);
