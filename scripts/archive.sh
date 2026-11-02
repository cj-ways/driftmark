#!/bin/sh
# Packs one release archive: the program with its licences and THIRD-PARTY-NOTICES,
# all at the top of the archive, as the GitHub Action, the GitLab template and the
# Homebrew formula expect them.
#   scripts/archive.sh <target> <out-dir>
# Reads the program from $CARGO_TARGET_DIR (default target)/<target>/release/ and
# THIRD-PARTY-NOTICES from the repository root. Writes <out-dir>/driftmark-<target>.tar.gz,
# or .zip for Windows (with 7-Zip, which Windows runners have).
set -eu
target=${1:?usage: scripts/archive.sh <target> <out-dir>}
out=${2:?usage: scripts/archive.sh <target> <out-dir>}
root=$(cd "$(dirname "$0")/.." && pwd)
case "$target" in
  *-windows-*) exe=driftmark.exe ;;
  *) exe=driftmark ;;
esac
bin="${CARGO_TARGET_DIR:-$root/target}/$target/release/$exe"
[ -f "$bin" ] || { echo "no program at $bin; build it first" >&2; exit 1; }
[ -f "$root/THIRD-PARTY-NOTICES" ] || {
  echo "no THIRD-PARTY-NOTICES; run scripts/notices/third-party-notices.mjs first" >&2
  exit 1
}

mkdir -p "$out"
out=$(cd "$out" && pwd)
stage="$out/.stage-$target"
mkdir -p "$stage"
cp "$bin" "$root/LICENSE-MIT" "$root/LICENSE-APACHE" "$root/THIRD-PARTY-NOTICES" "$stage/"
files="$exe LICENSE-MIT LICENSE-APACHE THIRD-PARTY-NOTICES"
case "$target" in
  *-windows-*)
    archive="$out/driftmark-$target.zip"
    rm -f "$archive"
    # A relative path: 7-Zip is a Windows program and does not read /d/... paths.
    # shellcheck disable=SC2086 # four plain file names
    (cd "$stage" && 7z a -tzip -bso0 -bsp0 "../driftmark-$target.zip" $files)
    ;;
  *)
    archive="$out/driftmark-$target.tar.gz"
    chmod 755 "$stage/$exe"
    # shellcheck disable=SC2086 # four plain file names
    tar -czf "$archive" -C "$stage" $files
    ;;
esac
for f in $files; do rm -f "$stage/$f"; done
rmdir "$stage"
echo "$archive: $(wc -c <"$archive" | tr -d ' ') bytes"
