#!/usr/bin/env bash
# Packages a release build for one target into dist/.
#   unix:    fss-mqtt-<version>-<target>.tar.gz (one top-level directory)
#   windows: fss-mqtt-<version>-<target>.zip   (flat, for Scoop/winget)
# Usage: scripts/package.sh <target> <version>
set -euo pipefail
target=$1
version=$2
name="fss-mqtt-${version}-${target}"
root=$(cd "$(dirname "$0")/.." && pwd)
mkdir -p "$root/dist"
stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT

case "$target" in
*windows*)
    cp "$root/target/$target/release/fss-mqtt.exe" "$root/README.md" "$root"/LICENSE-* "$stage/"
    (cd "$stage" && 7z a -tzip -bso0 "$root/dist/$name.zip" ./*)
    out="$name.zip"
    ;;
*)
    mkdir "$stage/$name"
    cp "$root/target/$target/release/fss-mqtt" "$root/README.md" "$root"/LICENSE-* "$stage/$name/"
    tar -C "$stage" -czf "$root/dist/$name.tar.gz" "$name"
    out="$name.tar.gz"
    ;;
esac
echo "dist/$out"
