#!/usr/bin/env bash
# Builds fss-mqtt's pacman repository for a release: the package for x86_64
# and aarch64 from the release's static Linux builds, and each architecture's
# database. Runs in an Arch Linux container (archlinux:base-devel), as root.
#   build-repo.sh <version> <SHA256SUMS file> <out dir>
# Out: <out>/x86_64/ and <out>/aarch64/, each with fss-mqtt.db (and .files)
# and the package. With GPGKEY set (a key in root's keyring, without a
# passphrase), packages and databases are signed, and <out>/fss-mqtt.asc is
# the public key.
set -euo pipefail
version=$1 sums=$(realpath "$2") out=$(realpath -m "$3")
here=$(cd "$(dirname "$0")" && pwd)
sum() { awk -v f="fss-mqtt-$version-$1-unknown-linux-musl.tar.gz" '$2 == f { print $1 }' "$sums"; }
x86=$(sum x86_64) arm=$(sum aarch64)
[ -n "$x86" ] && [ -n "$arm" ] || { echo "no checksums for $version in $sums" >&2; exit 1; }

# makepkg won't run as root: a builder, with the recipe for this release.
id builder >/dev/null 2>&1 || useradd -m builder
work=$(mktemp -d)
sed -e "s/^pkgver=.*/pkgver=$version/" \
    -e "s/^sha256sums_x86_64=.*/sha256sums_x86_64=('$x86')/" \
    -e "s/^sha256sums_aarch64=.*/sha256sums_aarch64=('$arm')/" \
    "$here/PKGBUILD" > "$work/PKGBUILD"
chown -R builder "$work"

sign=()
if [ -n "${GPGKEY:-}" ]; then
    # The builder signs with the key, imported into a keyring of its own.
    gpg --batch --export-secret-keys "$GPGKEY" | su builder -c "HOME=/home/builder gpg --batch --import"
    sign=(--sign --key "$GPGKEY")
fi

mkdir -p "$out"
for arch in x86_64 aarch64; do
    # Nothing is compiled: the aarch64 package is made here as well, by
    # telling makepkg that's the machine.
    conf="$work/makepkg-$arch.conf"
    sed "s/^CARCH=.*/CARCH=\"$arch\"/" /etc/makepkg.conf > "$conf"
    echo "PKGDEST=$work/$arch" >> "$conf"
    mkdir -p "$work/$arch" && chown builder "$work/$arch"
    (cd "$work" && su builder -c "HOME=/home/builder makepkg --config '$conf' --clean --cleanbuild --force ${sign[*]}")
    mkdir -p "$out/$arch"
    cp "$work/$arch"/*.pkg.tar.zst* "$out/$arch/"
    if [ ${#sign[@]} -gt 0 ]; then
        repo-add --sign --key "$GPGKEY" "$out/$arch/fss-mqtt.db.tar.gz" "$out/$arch"/*.pkg.tar.zst
    else
        repo-add "$out/$arch/fss-mqtt.db.tar.gz" "$out/$arch"/*.pkg.tar.zst
    fi
    # GitHub Pages serves no symlinks: the database under its own names.
    for f in db files db.sig files.sig; do
        link="$out/$arch/fss-mqtt.$f"
        [ -L "$link" ] && cp --remove-destination "$(readlink -f "$link")" "$link"
    done
done
trust="SigLevel = Optional TrustAll"
if [ -n "${GPGKEY:-}" ]; then
    gpg --armor --export "$GPGKEY" > "$out/fss-mqtt.asc"
    trust=""
fi

# A page for whoever opens the repository's address.
base="https://mbilling.github.io/fss-mqtt/arch"
cat > "$out/index.html" <<HTML
<!doctype html>
<meta charset="utf-8">
<title>fss-mqtt pacman repository</title>
<style>body{font:16px/1.5 ui-monospace,Menlo,monospace;max-width:46em;margin:3em auto;padding:0 1em}pre{background:#f4f4f4;padding:1em;overflow-x:auto}</style>
<h1>fss-mqtt for Arch Linux</h1>
<p>fss-mqtt $version for x86_64 and aarch64 (Arch Linux ARM), from the
<a href="https://github.com/mbilling/fss-mqtt">GitHub release</a>.</p>
$(if [ -n "${GPGKEY:-}" ]; then cat <<KEY
<p>Trust the key packages and databases are signed with (once):</p>
<pre>curl -fsSL $base/fss-mqtt.asc | sudo pacman-key --add -
sudo pacman-key --lsign-key $GPGKEY</pre>
KEY
fi)
<p>Add the repository to <code>/etc/pacman.conf</code>:</p>
<pre>[fss-mqtt]
$trust${trust:+
}Server = $base/\$arch</pre>
<p>Then <code>sudo pacman -Sy fss-mqtt</code>, and updates come with <code>pacman -Syu</code>.</p>
HTML
find "$out" -type f | sort
