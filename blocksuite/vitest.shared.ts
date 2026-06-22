import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

import { vanillaExtractPlugin } from '@vanilla-extract/vite-plugin';
import swc from 'unplugin-swc';
import { mergeConfig, type UserWorkspaceConfig } from 'vitest/config';

const repoRoot = fileURLToPath(new URL('..', import.meta.url));

const sharedTransformConfig: UserWorkspaceConfig = {
  esbuild: false,
  // Let SWC handle Stage 3 decorators consistently for Blocksuite sources.
  // OXC's default transform path currently changes field/decorator init order.
  oxc: false,
  plugins: [
    vanillaExtractPlugin(),
    swc.vite({
      jsc: {
        preserveAllComments: true,
        parser: {
          syntax: 'typescript',
          dynamicImport: true,
          tsx: true,
          decorators: true,
        },
        target: 'es2022',
        externalHelpers: false,
        transform: {
          react: {
            runtime: 'automatic',
          },
          useDefineForClassFields: false,
          decoratorVersion: '2022-03',
        },
      },
      sourceMaps: true,
      inlineSourcesContent: true,
    }),
  ],
  assetsInclude: ['**/*.md', '**/*.zip'],
  resolve: {
    alias: {
      yjs: resolve(repoRoot, 'node_modules/yjs'),
    },
  },
};

const browserProjectConfig: UserWorkspaceConfig = {
  test: {
    setupFiles: [resolve(repoRoot, 'scripts/vitest-blocksuite-polyfill.ts')],
    server: {
      deps: {
        inline: [/^@blocksuite\//],
      },
    },
  },
};

const nonBrowserProjectConfig: UserWorkspaceConfig = {
  test: {
    setupFiles: [
      resolve(repoRoot, 'scripts/vitest-blocksuite-polyfill.ts'),
      resolve(repoRoot, 'scripts/vitest-blocksuite-lit.ts'),
    ],
    server: {
      deps: {
        inline: [/^@blocksuite\//],
      },
    },
  },
};

export function defineBlocksuiteProject(config: UserWorkspaceConfig) {
  return mergeConfig(
    mergeConfig(sharedTransformConfig, nonBrowserProjectConfig),
    config
  );
}

export function defineBlocksuiteBrowserProject(config: UserWorkspaceConfig) {
  return mergeConfig(
    mergeConfig(sharedTransformConfig, browserProjectConfig),
    config
  );
}
