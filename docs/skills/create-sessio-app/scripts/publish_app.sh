#!/usr/bin/env bash
set -euo pipefail

usage() {
  printf 'Usage: %s <source-app-dir> <app-slug> [--force]\n' "$0" >&2
}

if [[ $# -lt 2 || $# -gt 3 ]]; then
  usage
  exit 64
fi

source_dir=$1
app_slug=$2
force=${3:-}

if [[ -z "${SESSIO_APP_HOME:-}" ]]; then
  printf 'SESSIO_APP_HOME is not set; refusing to guess a Sessio profile.\n' >&2
  exit 78
fi
if [[ "$SESSIO_APP_HOME" != /* ]]; then
  printf 'SESSIO_APP_HOME must be an absolute path.\n' >&2
  exit 78
fi
if [[ ! -d "$source_dir" ]]; then
  printf 'Source app directory does not exist: %s\n' "$source_dir" >&2
  exit 66
fi
if [[ ! "$app_slug" =~ ^[a-z0-9]+([.-][a-z0-9]+)*$ ]]; then
  printf 'App slug must use lowercase ASCII segments: %s\n' "$app_slug" >&2
  exit 64
fi
if [[ "$force" != "" && "$force" != "--force" ]]; then
  usage
  exit 64
fi

source_dir=$(cd "$source_dir" && pwd -P)
apps_dir="$SESSIO_APP_HOME/apps"
destination="$apps_dir/$app_slug"

if [[ -e "$destination" && "$force" != "--force" ]]; then
  printf 'Destination already exists; inspect it or rerun with --force: %s\n' "$destination" >&2
  exit 73
fi

mkdir -p "$apps_dir"
staging=$(mktemp -d "$apps_dir/.$app_slug.publish.XXXXXX")
cleanup() {
  rm -rf "$staging"
}
trap cleanup EXIT

cp -R "$source_dir"/. "$staging"/

if [[ -f "$staging/AGENTS.md" ]]; then
  cp "$staging/AGENTS.md" "$staging/CLAUDE.md"
fi

if [[ "$force" == "--force" && -e "$destination" ]]; then
  rm -rf "$destination"
fi
mv "$staging" "$destination"
trap - EXIT
printf '%s\n' "$destination"
