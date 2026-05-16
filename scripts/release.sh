#!/bin/sh
# Bump version, commit, tag.
#
# Usage:
#   ./scripts/release.sh 0.2.0
#
# Touches:
#   package.json                  "version": "..."
#   src-tauri/Cargo.toml          version = "..."
#   src-tauri/tauri.conf.json     "version": "..."
#   src-tauri/Cargo.lock          via `cargo check`
#
# Then commits + tags `vX.Y.Z` locally. Does NOT push; review and push:
#   git push origin <branch> vX.Y.Z

set -eu

root="$(cd "$(dirname "$0")/.." && pwd)"
package_json="$root/package.json"
cargo_toml="$root/src-tauri/Cargo.toml"
tauri_conf="$root/src-tauri/tauri.conf.json"

release_tag="$(git -C "$root" tag --list 'v*' --sort=-v:refname | head -n 1)"

if [ $# -eq 0 ]; then
    cat >&2 <<EOF
missing version
latest release tag: ${release_tag:-<none>}
usage: $0 <new-version>     e.g. $0 0.2.0
EOF
    exit 2
fi

if [ $# -ne 1 ]; then
    echo "usage: $0 <new-version>     e.g. $0 0.2.0" >&2
    exit 2
fi

new="$1"
if ! printf '%s' "$new" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+$'; then
    echo "version must look like X.Y.Z (got '$new')" >&2
    exit 2
fi

if [ -n "$release_tag" ]; then
    latest_release_version="${release_tag#v}"
    if ! printf '%s' "$latest_release_version" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+$'; then
        echo "latest release tag '$release_tag' is not in vX.Y.Z format" >&2
        exit 1
    fi

    if awk -v new="$new" -v latest="$latest_release_version" '
        BEGIN {
            split(new, a, ".")
            split(latest, b, ".")
            for (i = 1; i <= 3; i++) {
                if ((a[i] + 0) < (b[i] + 0)) exit 0
                if ((a[i] + 0) > (b[i] + 0)) exit 1
            }
            exit 0
        }
    '; then
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
