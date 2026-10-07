#!/usr/bin/env bash
# Prints a Scoop manifest for a release.
# Usage: scripts/scoop-manifest.sh <version> <owner/repo> <SHA256SUMS file>
set -euo pipefail
version=$1
repo=$2
sums=$3
f="fss-mqtt-$version-x86_64-pc-windows-msvc.zip"
hash=$(awk -v f="$f" '$2 == f || $2 == "*" f { print $1 }' "$sums")
[ -n "$hash" ] || { echo "no checksum for $f in $sums" >&2; exit 1; }

cat <<JSON
{
    "version": "$version",
    "description": "Fast, keyboard-driven MQTT v5 explorer for the terminal",
    "homepage": "https://github.com/$repo",
    "license": "MIT|Apache-2.0",
    "architecture": {
        "64bit": {
            "url": "https://github.com/$repo/releases/download/v$version/$f",
            "hash": "$hash"
        }
    },
    "bin": "fss-mqtt.exe",
    "checkver": "github",
    "autoupdate": {
        "architecture": {
            "64bit": {
                "url": "https://github.com/$repo/releases/download/v\$version/fss-mqtt-\$version-x86_64-pc-windows-msvc.zip"
            }
        }
    }
}
JSON
