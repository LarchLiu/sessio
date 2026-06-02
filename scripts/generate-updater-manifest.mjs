#!/usr/bin/env node
import { existsSync, readFileSync, writeFileSync } from "node:fs";
import { basename, join } from "node:path";

const args = new Map();
for (let i = 2; i < process.argv.length; i += 2) {
  const key = process.argv[i];
  const value = process.argv[i + 1];
  if (!key?.startsWith("--") || value === undefined) {
    throw new Error("usage: generate-updater-manifest --dir <dir> --tag <tag> --repo <owner/repo>");
  }
  args.set(key.slice(2), value);
}

const dir = args.get("dir") ?? "staged";
const tag = args.get("tag") ?? process.env.GITHUB_REF_NAME;
const repo = args.get("repo") ?? process.env.GITHUB_REPOSITORY;

if (!tag) throw new Error("missing release tag");
if (!repo) throw new Error("missing GitHub repository");

const version = tag.replace(/^v/i, "");
const baseUrl = `https://github.com/${repo}/releases/download/${tag}`;
const notesPath = args.get("notes") ?? join(dir, "CHANGELOG.md");

const packages = [
  {
    file: "sessio-macos-universal.app.tar.gz",
    targets: [
      "darwin-aarch64",
      "darwin-aarch64-app",
      "darwin-x86_64",
      "darwin-x86_64-app",
    ],
  },
  {
    file: "sessio-linux-x86_64.AppImage",
    targets: ["linux-x86_64", "linux-x86_64-appimage"],
  },
  {
    file: "sessio-linux-aarch64.AppImage",
    targets: ["linux-aarch64", "linux-aarch64-appimage"],
  },
  {
    file: "sessio-windows-x86_64-setup.exe",
    targets: ["windows-x86_64", "windows-x86_64-nsis"],
  },
];

const platforms = {};

for (const pkg of packages) {
  const assetPath = join(dir, pkg.file);
  const sigPath = `${assetPath}.sig`;
  if (!existsSync(assetPath)) throw new Error(`missing updater asset: ${assetPath}`);
  if (!existsSync(sigPath)) throw new Error(`missing updater signature: ${sigPath}`);

  const entry = {
    url: `${baseUrl}/${encodeURIComponent(basename(pkg.file))}`,
    signature: readFileSync(sigPath, "utf8").trim(),
  };
  for (const target of pkg.targets) {
    platforms[target] = entry;
  }
}

const manifest = {
  version,
  notes: existsSync(notesPath) ? readFileSync(notesPath, "utf8").trim() : "",
  pub_date: new Date().toISOString(),
  platforms,
};

writeFileSync(join(dir, "latest.json"), `${JSON.stringify(manifest, null, 2)}\n`);
