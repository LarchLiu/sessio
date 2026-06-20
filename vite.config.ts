import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import pkg from "./package.json" with { type: "json" };

const host = process.env.TAURI_DEV_HOST;
const rootDir = fileURLToPath(new URL(".", import.meta.url));
const blockSuiteIconsLitRequest = "@blocksuite/icons/lit";
const blockSuiteIconsLitCompatId = "\0blocksuite-icons-lit-compat";

function findBlockSuiteIconsLitPath() {
  const pnpmDir = path.join(rootDir, "node_modules", ".pnpm");
  const packageDir = fs
    .readdirSync(pnpmDir)
    .sort()
    .find(name => name.startsWith("@blocksuite+icons@"));

  if (!packageDir) {
    throw new Error("Unable to locate @blocksuite/icons in node_modules/.pnpm");
  }

  return path.join(
    pnpmDir,
    packageDir,
    "node_modules",
    "@blocksuite",
    "icons",
    "dist",
    "lit.mjs",
  );
}

const blockSuiteIconsLitPath = findBlockSuiteIconsLitPath();

function createBlockSuiteIconsCompatCode() {
  const source = fs.readFileSync(blockSuiteIconsLitPath, "utf8");

  return `${source}
export { CheckBoxCheckSolid as CheckBoxCkeckSolidIcon };
`;
}

function blockSuiteIconsCompatPlugin() {
  return {
    name: "blocksuite-icons-lit-compat",
    enforce: "pre" as const,
    resolveId(source: string) {
      if (source === blockSuiteIconsLitRequest) {
        return blockSuiteIconsLitCompatId;
      }
      return null;
    },
    load(id: string) {
      if (id === blockSuiteIconsLitCompatId) {
        return createBlockSuiteIconsCompatCode();
      }
      return null;
    },
  };
}

export default defineConfig(async () => ({
  plugins: [blockSuiteIconsCompatPlugin(), react()],
  clearScreen: false,
  optimizeDeps: {
    esbuildOptions: {
      plugins: [
        {
          name: "blocksuite-icons-lit-compat",
          setup(build) {
            build.onResolve({ filter: /^@blocksuite\/icons\/lit$/ }, () => ({
              path: blockSuiteIconsLitCompatId,
              namespace: "blocksuite-icons-lit-compat",
            }));
            build.onLoad({ filter: /.*/, namespace: "blocksuite-icons-lit-compat" }, () => ({
              contents: createBlockSuiteIconsCompatCode(),
              loader: "js",
            }));
          },
        },
      ],
    },
  },
  define: {
    __APP_VERSION__: JSON.stringify(pkg.version),
    __APP_IS_DEV__: JSON.stringify(
      process.env.SESSIO_APP_VARIANT === "dev" || process.env.NODE_ENV !== "production",
    ),
  },
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host
      ? { protocol: "ws", host, port: 1421 }
      : undefined,
    watch: { ignored: ["**/src-tauri/**"] },
  },
}));
