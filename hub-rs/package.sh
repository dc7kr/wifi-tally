#!/usr/bin/env bash
# Builds the web frontend and the Rust hub and bundles both into one archive:
#
#   dist/vtally-hub-<version>-<target>/
#       vtally-hub            the executable
#       frontend/             the built React frontend (found automatically next to the executable)
#       Readme.md, LICENSE
#
# Usage: ./package.sh [--target <rust-target-triple>] [--version <name>] [--skip-frontend]
#   --target         cross compile (the target must be installed: rustup target add <triple>)
#   --version        name used in the archive, default: `git describe`, or the Cargo.toml version
#   --skip-frontend  reuse an existing ../frontend/build instead of running npm
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
frontend_dir="$here/../frontend"
target=""
version=""
skip_frontend=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    --target) target="${2:?--target needs a value}"; shift 2 ;;
    --version) version="${2:?--version needs a value}"; shift 2 ;;
    --skip-frontend) skip_frontend=1; shift ;;
    -h|--help) sed -n '2,13p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "Unknown argument: $1" >&2; exit 1 ;;
  esac
done

if [[ -z "$version" ]]; then
  version="$(git -C "$here" describe --tags --always 2>/dev/null || true)"
  [[ -n "$version" ]] || version="$(sed -n 's/^version *= *"\(.*\)"/\1/p' "$here/Cargo.toml" | head -1)"
fi

host="$(rustc -vV | sed -n 's/^host: //p')"
triple="${target:-$host}"
exe="vtally-hub"
[[ "$triple" == *windows* ]] && exe="vtally-hub.exe"

# 1. frontend
if [[ $skip_frontend -eq 0 ]]; then
  echo "==> Building the frontend"
  (cd "$frontend_dir" && npm ci --ignore-scripts && npm run build)
fi
[[ -f "$frontend_dir/build/index.html" ]] || { echo "Frontend not found in $frontend_dir/build" >&2; exit 1; }

# 2. backend
echo "==> Building the hub for $triple"
build_args=(build --release --manifest-path "$here/Cargo.toml")
[[ -n "$target" ]] && build_args+=(--target "$target")
cargo "${build_args[@]}"
if [[ -n "$target" ]]; then
  bin="$here/target/$target/release/$exe"
else
  bin="$here/target/release/$exe"
fi
[[ -f "$bin" ]] || { echo "Executable not found: $bin" >&2; exit 1; }

# 3. bundle
name="vtally-hub-$version-$triple"
out="$here/dist/$name"
echo "==> Packaging $name"
rm -rf "$out" "$here/dist/$name.tar.gz" "$here/dist/$name.zip"
mkdir -p "$out"
cp "$bin" "$out/$exe"
cp -R "$frontend_dir/build" "$out/frontend"
cp "$here/Readme.md" "$out/Readme.md"
[[ -f "$here/../LICENSE" ]] && cp "$here/../LICENSE" "$out/LICENSE"

cd "$here/dist"
if [[ "$triple" == *windows* ]]; then
  if command -v zip >/dev/null; then
    zip -qr "$name.zip" "$name"
  elif command -v 7z >/dev/null; then
    7z a -bso0 -bsp0 "$name.zip" "$name"
  else
    echo "Neither zip nor 7z found, leaving $out" >&2; exit 0
  fi
  archive="$name.zip"
else
  tar -czf "$name.tar.gz" "$name"
  archive="$name.tar.gz"
fi

echo "==> Done: $here/dist/$archive"
echo "    Run: tar -xzf $archive && ./$name/$exe   (serves ./$name/frontend on port 3000)"
