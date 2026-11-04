#!/bin/sh
# 公開の前に、版がそろっていることを確かめる。
#   usage: sh scripts/check-release-version.sh [v0.1.0]
# 引数（git のタグ）を渡すと、それとも突き合わせる。渡さなければ Cargo.toml と package.json だけ見る。
# Cargo.toml の [workspace.package] の version と package.json の version が同じで、
# 引数があればタグから先頭の v を除いたものとも同じなら 0 で終わる。
set -eu

cd "$(dirname "$0")/.."

cargo_version=$(sed -n '/^\[workspace\.package\]/,/^\[/{s/^version *= *"\(.*\)"/\1/p;}' Cargo.toml)
npm_version=$(sed -n 's/^  "version": "\(.*\)",/\1/p' package.json)

echo "Cargo.toml   : ${cargo_version}"
echo "package.json : ${npm_version}"

if [ -z "$cargo_version" ] || [ "$cargo_version" != "$npm_version" ]; then
  echo "error: Cargo.toml と package.json の version が違う（または読めない）" >&2
  exit 1
fi

if [ $# -ge 1 ]; then
  tag=$1
  echo "tag          : ${tag}"
  case "$tag" in
    v[0-9]*) ;;
    *)
      echo "error: タグは v0.1.0 の形にする" >&2
      exit 1
      ;;
  esac
  if [ "${tag#v}" != "$cargo_version" ]; then
    echo "error: タグ ${tag} と version ${cargo_version} が違う" >&2
    exit 1
  fi
fi
echo "ok"
