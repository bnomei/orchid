#!/usr/bin/env bash
set -euo pipefail

# Reject generated and platform-specific artifacts that should not be present in
# the repository checkout or in crates.io source packages.
readonly forbidden_name_regex='(^|/)(\.DS_Store|\.AppleDouble|\.LSOverride|Icon\?|\._[^/]*|\.Spotlight-V100|\.Trashes|\.fseventsd|Thumbs\.db|Desktop\.ini|ehthumbs\.db)$'
readonly forbidden_ext_regex='(~|\.swp|\.swo|\.tmp|\.temp|\.bak|\.orig|\.rej|\.log)$'

check_stream() {
  local label="$1"
  local found=0

  while IFS= read -r path; do
    # Cargo includes Cargo.toml.orig as package metadata so crates.io can show
    # the author's original manifest alongside Cargo's normalized manifest.
    if [[ "$label" == "cargo package" && "$path" == "Cargo.toml.orig" ]]; then
      continue
    fi

    if [[ "$path" =~ $forbidden_name_regex || "$path" =~ $forbidden_ext_regex ]]; then
      printf '%s contains generated/platform artifact: %s\n' "$label" "$path" >&2
      found=1
    fi
  done

  return "$found"
}

workspace_files() {
  find . \
    -path ./.git -prune -o \
    -path ./target -prune -o \
    -type f -print \
    | sed 's#^./##'
}

package_files() {
  cargo package --locked --allow-dirty --list
}

workspace_files | check_stream "workspace"
package_files | check_stream "cargo package"
