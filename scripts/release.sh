#!/bin/sh
# Bump version, commit, tag.
#
# Usage:
#   ./scripts/release.sh 0.2.0
#   ./scripts/release.sh 0.3.0-beta.1
#
# Touches:
#   package.json                  "version": "..."
#   src-tauri/Cargo.toml          version = "..."
#   src-tauri/tauri.conf.json     "version": "..."
#   src-tauri/Cargo.lock          via `cargo check`
#
# Then commits + tags `vX.Y.Z[-prerelease]` locally. Does NOT push; review and push:
#   git push origin <branch> vX.Y.Z[-prerelease]

set -eu

root="$(cd "$(dirname "$0")/.." && pwd)"
package_json="$root/package.json"
cargo_toml="$root/src-tauri/Cargo.toml"
tauri_conf="$root/src-tauri/tauri.conf.json"

release_tag="$(
    git -C "$root" tag --list 'v*' | node -e '
const fs = require("node:fs");
const tags = fs.readFileSync(0, "utf8").trim().split(/\n+/).filter(Boolean);
function parse(tag) {
  const value = tag.replace(/^v/i, "");
  const match = value.match(/^(\d+)\.(\d+)\.(\d+)(?:-([0-9A-Za-z]+(?:[.-][0-9A-Za-z]+)*))?$/);
  if (!match) return null;
  return { tag, major: +match[1], minor: +match[2], patch: +match[3], prerelease: match[4] ?? "" };
}
function cmpPrerelease(a, b) {
  if (!a && !b) return 0;
  if (!a) return 1;
  if (!b) return -1;
  const ap = a.split(".");
  const bp = b.split(".");
  const n = Math.max(ap.length, bp.length);
  for (let i = 0; i < n; i++) {
    if (ap[i] === undefined) return -1;
    if (bp[i] === undefined) return 1;
    const an = /^\d+$/.test(ap[i]);
    const bn = /^\d+$/.test(bp[i]);
    if (an && bn) {
      const diff = Number(ap[i]) - Number(bp[i]);
      if (diff) return diff < 0 ? -1 : 1;
    } else if (an !== bn) {
      return an ? -1 : 1;
    } else if (ap[i] !== bp[i]) {
      return ap[i] < bp[i] ? -1 : 1;
    }
  }
  return 0;
}
function cmp(a, b) {
  for (const key of ["major", "minor", "patch"]) {
    if (a[key] !== b[key]) return a[key] - b[key];
  }
  return cmpPrerelease(a.prerelease, b.prerelease);
}
const parsed = tags.map(parse).filter(Boolean).sort(cmp);
if (parsed.length) process.stdout.write(parsed[parsed.length - 1].tag);
'
)"

if [ $# -eq 0 ]; then
    cat >&2 <<EOF
missing version
latest release tag: ${release_tag:-<none>}
usage: $0 <new-version>     e.g. $0 0.2.0 or $0 0.3.0-beta.1
EOF
    exit 2
fi

if [ $# -ne 1 ]; then
    echo "usage: $0 <new-version>     e.g. $0 0.2.0 or $0 0.3.0-beta.1" >&2
    exit 2
fi

new="$1"
if ! printf '%s' "$new" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z]+([.-][0-9A-Za-z]+)*)?$'; then
    echo "version must look like X.Y.Z or X.Y.Z-prerelease (got '$new')" >&2
    exit 2
fi

if [ -n "$release_tag" ]; then
    latest_release_version="${release_tag#v}"
    if ! printf '%s' "$latest_release_version" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z]+([.-][0-9A-Za-z]+)*)?$'; then
        echo "latest release tag '$release_tag' is not in vX.Y.Z[-prerelease] format" >&2
        exit 1
    fi

    if ! node - "$new" "$latest_release_version" <<'NODE'
const [next, latest] = process.argv.slice(2);
function parse(value) {
  const match = value.match(/^(\d+)\.(\d+)\.(\d+)(?:-([0-9A-Za-z]+(?:[.-][0-9A-Za-z]+)*))?$/);
  if (!match) throw new Error(`invalid version: ${value}`);
  return { major: +match[1], minor: +match[2], patch: +match[3], prerelease: match[4] ?? "" };
}
function cmpPrerelease(a, b) {
  if (!a && !b) return 0;
  if (!a) return 1;
  if (!b) return -1;
  const ap = a.split(".");
  const bp = b.split(".");
  const n = Math.max(ap.length, bp.length);
  for (let i = 0; i < n; i++) {
    if (ap[i] === undefined) return -1;
    if (bp[i] === undefined) return 1;
    const an = /^\d+$/.test(ap[i]);
    const bn = /^\d+$/.test(bp[i]);
    if (an && bn) {
      const diff = Number(ap[i]) - Number(bp[i]);
      if (diff) return diff < 0 ? -1 : 1;
    } else if (an !== bn) {
      return an ? -1 : 1;
    } else if (ap[i] !== bp[i]) {
      return ap[i] < bp[i] ? -1 : 1;
    }
  }
  return 0;
}
function cmp(a, b) {
  for (const key of ["major", "minor", "patch"]) {
    if (a[key] !== b[key]) return a[key] - b[key];
  }
  return cmpPrerelease(a.prerelease, b.prerelease);
}
process.exit(cmp(parse(next), parse(latest)) > 0 ? 0 : 1);
NODE
    then
        echo "version '$new' must be greater than latest release tag '$release_tag'" >&2
        exit 1
    fi
fi

# Refuse if the working tree has uncommitted work; the bump commit below
# should contain only release version metadata.
if ! git -C "$root" diff --quiet || ! git -C "$root" diff --cached --quiet || [ -n "$(git -C "$root" ls-files --others --exclude-standard)" ]; then
    echo "working tree has uncommitted changes; commit or stash first" >&2
    exit 1
fi

# Refuse if the tag already exists; otherwise `git tag` fails after the
# bump commit lands, leaving a commit to clean up by hand.
if [ -n "$(git -C "$root" tag -l "v$new")" ]; then
    echo "tag v$new already exists" >&2
    exit 1
fi

# perl is available on macOS, Linux, and Git-Bash; it avoids sed -i variants.
perl -pi -e 's/"version":\s*".*"/"version": "'"$new"'"/' "$package_json"
perl -pi -e 's/^version = ".*"/version = "'"$new"'"/' "$cargo_toml"
perl -pi -e 's/"version":\s*".*"/"version": "'"$new"'"/' "$tauri_conf"

# Refresh Cargo.lock so the commit is self-consistent.
( cd "$root/src-tauri" && cargo check )

git -C "$root" add \
    package.json \
    src-tauri/Cargo.toml \
    src-tauri/tauri.conf.json \
    src-tauri/Cargo.lock
git -C "$root" commit -m "chore: release v$new"
git -C "$root" tag "v$new"

branch="$(git -C "$root" rev-parse --abbrev-ref HEAD)"
cat <<EOF

release v$new staged on branch '$branch'.
to publish (triggers .github/workflows/release.yml):
    git push origin $branch v$new
EOF
