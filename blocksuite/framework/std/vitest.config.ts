import { playwright } from '@vitest/browser-playwright';
import { defineConfig } from 'vitest/config';

import { defineBlocksuiteBrowserProject } from '../../vitest.shared';

export default defineConfig(defineBlocksuiteBrowserProject({
  esbuild: {
    target: 'es2018',
  },
  test: {
    browser: {
      enabled: true,
      headless: true,
      instances: [{ browser: 'chromium' }],
      provider: playwright(),
      isolate: false,
    },
    include: ['src/__tests__/**/*.unit.spec.ts'],
    testTimeout: 500,
    coverage: {
      provider: 'istanbul',
      reporter: ['lcov'],
      reportsDirectory: '../../../.coverage/std',
    },
    restoreMocks: true,
  },
}));
