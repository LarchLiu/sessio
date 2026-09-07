#!/usr/bin/env bash
set -euo pipefail

usage() {
  printf 'Usage: %s <source-app-dir> <app-slug> [--update] [--update-data]\n' "$0" >&2
}

if [[ $# -lt 2 || $# -gt 4 ]]; then
  usage
  exit 64
fi

source_dir=$1
app_slug=$2
update=false
update_data=false
for option in "${@:3}"; do
  case "$option" in
    --update) update=true ;;
    --update-data) update_data=true ;;
    *) usage; exit 64 ;;
  esac
done

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
if [[ "$update_data" == true ]]; then
  update=true
fi

source_dir=$(cd "$source_dir" && pwd -P)
apps_dir="$SESSIO_APP_HOME/apps"
destination="$apps_dir/$app_slug"

if [[ ( -e "$destination" || -L "$destination" ) && "$update" != true ]]; then
  printf 'Destination already exists; inspect it or rerun with --update: %s\n' "$destination" >&2
  exit 73
fi
if [[ "$update" == true && ( -e "$destination" || -L "$destination" ) ]]; then
  if [[ -L "$destination" || ! -d "$destination" ]]; then
    printf 'Existing destination must be a real directory: %s\n' "$destination" >&2
    exit 73
  fi
fi

copy_tree_merge() {
  local source=$1
  local target=$2
  local relative_path=${3:-}
  local entry
  local target_entry
  local entry_relative

  mkdir -p "$target"
  while IFS= read -r -d '' entry; do
    target_entry="$target/${entry##*/}"
    if [[ -d "$entry" && ! -L "$entry" ]]; then
      if [[ -e "$target_entry" || -L "$target_entry" ]]; then
        if [[ -L "$target_entry" || ! -d "$target_entry" ]]; then
          rm -rf "$target_entry"
          mkdir -p "$target_entry"
        fi
      else
        mkdir -p "$target_entry"
      fi
      entry_relative="${relative_path:+$relative_path/}${entry##*/}"
      copy_tree_merge "$entry" "$target_entry" "$entry_relative"
    else
      entry_relative="${relative_path:+$relative_path/}${entry##*/}"
      if [[ "$update_data" != true && "$entry_relative" == "web/$app_slug-data.js" && -f "$target_entry" ]]; then
        continue
      fi
      if [[ -e "$target_entry" || -L "$target_entry" ]]; then
        rm -rf "$target_entry"
      fi
      cp -P "$entry" "$target_entry"
    fi
  done < <(find "$source" -mindepth 1 -maxdepth 1 -print0)
}

write_claude_instructions() {
  local target=$1
  if [[ -f "$target/AGENTS.md" ]]; then
    cp "$target/AGENTS.md" "$target/CLAUDE.md"
  fi
}

mkdir -p "$apps_dir"
if [[ "$update" == true && -d "$destination" ]]; then
  copy_tree_merge "$source_dir" "$destination"
  write_claude_instructions "$destination"
  printf '%s\n' "$destination"
  exit 0
fi

staging=$(mktemp -d "$apps_dir/.$app_slug.publish.XXXXXX")
cleanup() {
  rm -rf "$staging"
}
trap cleanup EXIT

copy_tree_merge "$source_dir" "$staging"
write_claude_instructions "$staging"
mv "$staging" "$destination"
trap - EXIT
printf '%s\n' "$destination"
