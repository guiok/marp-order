#!/bin/sh
# Builds a .deb with the marp-order binary.
#   packaging/build-deb.sh <binary> <version> <output folder>
# Needs dpkg-deb, gzip and objdump (binutils). The architecture is that of the machine.
set -eu

binary=$1
version=$2
out=$3

arch=$(dpkg --print-architecture)
root=$(mktemp -d)
chmod 755 "$root"
trap 'rm -rf "$root"' EXIT

install -D -m 755 "$binary" "$root/usr/bin/marp-order"
install -D -m 644 README.md "$root/usr/share/doc/marp-order/README.md"
install -D -m 644 README.pt-BR.md "$root/usr/share/doc/marp-order/README.pt-BR.md"
install -D -m 644 LICENSE "$root/usr/share/doc/marp-order/copyright"
install -D -m 644 packaging/marp-order.1 "$root/usr/share/man/man1/marp-order.1"
gzip -9n "$root/usr/share/man/man1/marp-order.1"

# the changes of each version are listed in the GitHub releases
cat > "$root/usr/share/doc/marp-order/changelog" <<CHANGELOG
marp-order ($version) unstable; urgency=low

  * See https://github.com/guiok/marp-order/releases/tag/v$version

 -- Guilherme Kunz <29389316+guiok@users.noreply.github.com>  $(date -R)
CHANGELOG
gzip -9n "$root/usr/share/doc/marp-order/changelog"
chmod 644 "$root/usr/share/doc/marp-order/changelog.gz"

# the newest glibc symbol the binary uses is the oldest libc6 it runs on
glibc=$(objdump -T "$binary" | grep -o 'GLIBC_[0-9.]*' | sort -Vu | tail -1 | cut -d_ -f2)
size=$(du -sk "$root" | cut -f1)

mkdir "$root/DEBIAN"
cat > "$root/DEBIAN/control" <<CONTROL
Package: marp-order
Version: $version
Architecture: $arch
Maintainer: Guilherme Kunz <29389316+guiok@users.noreply.github.com>
Installed-Size: $size
Depends: libc6 (>= $glibc), libgcc-s1
Suggests: chromium | chromium-browser | google-chrome-stable, nodejs
Section: text
Priority: optional
Homepage: https://github.com/guiok/marp-order
Description: visual slide sorter for Marp presentations
 Reorder the slides of a Marp deck by dragging thumbnails around a grid in
 the browser, edit them with a live preview, compare and merge decks, and
 present with the speaker notes read aloud. The Markdown file is rewritten
 without reformatting the slides that were not edited.
 .
 Thumbnails, the live preview and the presentation need marp-cli
 (npm install -g @marp-team/marp-cli) and Chrome or Chromium.
CONTROL

mkdir -p "$out"
dpkg-deb --build --root-owner-group "$root" "$out/marp-order_${version}_${arch}.deb"
