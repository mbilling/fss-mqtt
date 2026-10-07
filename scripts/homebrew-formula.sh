#!/usr/bin/env bash
# Prints a Homebrew formula (macOS + Linux, arm64 + x86_64) for a release.
# Usage: scripts/homebrew-formula.sh <version> <owner/repo> <SHA256SUMS file>
set -euo pipefail
version=$1
repo=$2
sums=$3
base="https://github.com/$repo/releases/download/v$version"

sha() { # sha <target>
    local f="fss-mqtt-$version-$1.tar.gz"
    local s
    s=$(awk -v f="$f" '$2 == f || $2 == "*" f { print $1 }' "$sums")
    [ -n "$s" ] || { echo "no checksum for $f in $sums" >&2; exit 1; }
    echo "$s"
}

cat <<RUBY
class FssMqtt < Formula
  desc "Fast, keyboard-driven MQTT v5 explorer for the terminal"
  homepage "https://github.com/$repo"
  version "$version"
  license any_of: ["MIT", "Apache-2.0"]

  on_macos do
    on_arm do
      url "$base/fss-mqtt-$version-aarch64-apple-darwin.tar.gz"
      sha256 "$(sha aarch64-apple-darwin)"
    end
    on_intel do
      url "$base/fss-mqtt-$version-x86_64-apple-darwin.tar.gz"
      sha256 "$(sha x86_64-apple-darwin)"
    end
  end

  on_linux do
    on_arm do
      url "$base/fss-mqtt-$version-aarch64-unknown-linux-musl.tar.gz"
      sha256 "$(sha aarch64-unknown-linux-musl)"
    end
    on_intel do
      url "$base/fss-mqtt-$version-x86_64-unknown-linux-musl.tar.gz"
      sha256 "$(sha x86_64-unknown-linux-musl)"
    end
  end

  def install
    bin.install "fss-mqtt"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/fss-mqtt --version")
  end
end
RUBY
