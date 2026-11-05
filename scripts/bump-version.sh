#!/bin/sh
# 版を 1 段上げる。release.yml が使う。
#   usage: sh scripts/bump-version.sh <patch|minor|major>
# Cargo.toml の [workspace.package] の version を読み、指定の桁を上げて、
# Cargo.toml・Cargo.lock・package.json・package-lock.json を新しい版にする。新しい版を標準出力に出す。
set -eu

cd "$(dirname "$0")/.."

bump=${1:-}
current=$(sed -n '/^\[workspace\.package\]/,/^\[/{s/^version *= *"\([0-9]*\.[0-9]*\.[0-9]*\)"$/\1/p;}' Cargo.toml)
if [ -z "$current" ]; then
  echo "error: Cargo.toml の [workspace.package] から X.Y.Z の version を読めない" >&2
  exit 1
fi

IFS=. read -r major minor patch <<EOF
$current
EOF
case "$bump" in
  major) major=$((major + 1)); minor=0; patch=0 ;;
  minor) minor=$((minor + 1)); patch=0 ;;
  patch) patch=$((patch + 1)) ;;
  *)
    echo "error: 桁は patch・minor・major のどれか" >&2
    exit 1
    ;;
esac
next="$major.$minor.$patch"

# Cargo.toml: [workspace.package] の version だけ
sed -i.bak "/^\[workspace\.package\]/,/^\[/s/^version = \"$current\"$/version = \"$next\"/" Cargo.toml
# Cargo.lock: workspace の 2 つの crate の version だけ
sed -i.bak "/^name = \"coneko-term\"$/,/^version/s/^version = \"$current\"$/version = \"$next\"/; /^name = \"coneko-term-wasm\"$/,/^version/s/^version = \"$current\"$/version = \"$next\"/" Cargo.lock
rm -f Cargo.toml.bak Cargo.lock.bak
# package.json と package-lock.json
npm version "$next" --no-git-tag-version --ignore-scripts > /dev/null

echo "$next"
