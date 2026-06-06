#!/usr/bin/env node
import { createWriteStream, existsSync, mkdirSync, rmSync, statSync } from "node:fs";
import { chmod, copyFile, readdir } from "node:fs/promises";
import { basename, join, resolve } from "node:path";
import { pipeline } from "node:stream/promises";
import { spawnSync } from "node:child_process";
import { get } from "node:https";
import { fileURLToPath } from "node:url";

const DEFAULT_PI_VERSION = "v0.1.17";
const ROOT = resolve(fileURLToPath(new URL("..", import.meta.url)));
const BIN_DIR = join(ROOT, "src-tauri", "binaries");

const TARGETS = {
  "aarch64-apple-darwin": {
    archive: "pi-darwin-arm64.tar.xz",
    executable: "pi",
    output: "astra-pi-aarch64-apple-darwin",
  },
  "x86_64-apple-darwin": {
    archive: "pi-darwin-amd64.tar.xz",
    executable: "pi",
    output: "astra-pi-x86_64-apple-darwin",
  },
  "universal-apple-darwin": {
    output: "astra-pi-universal-apple-darwin",
    universal: ["aarch64-apple-darwin", "x86_64-apple-darwin"],
  },
  "x86_64-unknown-linux-gnu": {
    archive: "pi-linux-amd64.tar.xz",
    executable: "pi",
    output: "astra-pi-x86_64-unknown-linux-gnu",
  },
  "aarch64-unknown-linux-gnu": {
    archive: "pi-linux-arm64.tar.xz",
    executable: "pi",
    output: "astra-pi-aarch64-unknown-linux-gnu",
  },
  "x86_64-pc-windows-msvc": {
    archive: "pi-windows-amd64.zip",
    executable: "pi.exe",
    output: "astra-pi-x86_64-pc-windows-msvc.exe",
  },
};

function usage() {
  const targets = Object.keys(TARGETS).join(", ");
  console.error(`usage: node scripts/prepare-pi-sidecar.mjs <target-triple|all>\nknown targets: ${targets}`);
}

function run(command, args, options = {}) {
  const result = spawnSync(command, args, {
    stdio: "inherit",
    ...options,
  });
  if (result.error) {
    throw result.error;
  }
  if (result.status !== 0) {
    throw new Error(`${command} ${args.join(" ")} failed with exit code ${result.status}`);
  }
}

async function download(url, destination) {
  console.log(`downloading ${url}`);
  await new Promise((resolveDownload, reject) => {
    const request = get(url, (response) => {
      if (response.statusCode && response.statusCode >= 300 && response.statusCode < 400 && response.headers.location) {
        download(new URL(response.headers.location, url).toString(), destination)
          .then(resolveDownload, reject);
        return;
      }
      if (response.statusCode !== 200) {
        reject(new Error(`download failed: ${response.statusCode} ${response.statusMessage}`));
        response.resume();
        return;
      }
      pipeline(response, createWriteStream(destination)).then(resolveDownload, reject);
    });
    request.on("error", reject);
  });
}

async function ensureArchive(target) {
  mkdirSync(BIN_DIR, { recursive: true });
  const archivePath = join(BIN_DIR, target.archive);
  if (existsSync(archivePath)) {
    return archivePath;
  }

  const version = process.env.SESSIO_PI_AGENT_RUST_VERSION || DEFAULT_PI_VERSION;
  const baseUrl =
    process.env.SESSIO_PI_AGENT_RUST_RELEASE_BASE_URL ||
    `https://github.com/Dicklesworthstone/pi_agent_rust/releases/download/${version}`;
  await download(`${baseUrl}/${target.archive}`, archivePath);
  return archivePath;
}

async function findExtractedExecutable(dir, executable) {
  const entries = await readdir(dir, { recursive: true, withFileTypes: true });
  const matches = entries
    .filter((entry) => entry.isFile() && entry.name === executable)
    .map((entry) => join(entry.parentPath ?? dir, entry.name));
  if (matches.length === 0) {
    throw new Error(`archive did not contain ${executable}`);
  }
  matches.sort((left, right) => left.length - right.length);
  return matches[0];
}

async function prepareTarget(targetTriple) {
  const target = TARGETS[targetTriple];
  if (!target) {
    throw new Error(`unknown target: ${targetTriple}`);
  }
  if (target.universal) {
    for (const slice of target.universal) {
      await prepareTarget(slice);
    }
    const outputPath = join(BIN_DIR, target.output);
    run("lipo", [
      "-create",
      ...target.universal.map((slice) => join(BIN_DIR, TARGETS[slice].output)),
      "-output",
      outputPath,
    ]);
    await chmod(outputPath, 0o755);
    const size = statSync(outputPath).size;
    console.log(`prepared ${basename(outputPath)} (${size} bytes)`);
    return;
  }

  const archivePath = await ensureArchive(target);
  const tempDir = join(BIN_DIR, `.astra-pi-extract-${targetTriple}`);
  const outputPath = join(BIN_DIR, target.output);
  rmSync(tempDir, { recursive: true, force: true });
  mkdirSync(tempDir, { recursive: true });

  if (archivePath.endsWith(".zip")) {
    if (process.platform === "win32") {
      run("powershell", [
        "-NoProfile",
        "-ExecutionPolicy",
        "Bypass",
        "-Command",
        `Expand-Archive -LiteralPath ${JSON.stringify(archivePath)} -DestinationPath ${JSON.stringify(tempDir)} -Force`,
      ]);
    } else {
      run("unzip", ["-q", archivePath, "-d", tempDir]);
    }
  } else {
    run("tar", ["-xf", archivePath, "-C", tempDir]);
  }

  const extracted = await findExtractedExecutable(tempDir, target.executable);
  await copyFile(extracted, outputPath);
  if (!target.output.endsWith(".exe")) {
    await chmod(outputPath, 0o755);
  }
  const size = statSync(outputPath).size;
  rmSync(tempDir, { recursive: true, force: true });
  console.log(`prepared ${basename(outputPath)} (${size} bytes)`);
}

const requested = process.argv.slice(2);
if (requested.length !== 1) {
  usage();
  process.exit(2);
}

const targets = requested[0] === "all" ? Object.keys(TARGETS) : requested;

try {
  for (const target of targets) {
    await prepareTarget(target);
  }
} catch (error) {
  console.error(error instanceof Error ? error.message : String(error));
  process.exit(1);
}
