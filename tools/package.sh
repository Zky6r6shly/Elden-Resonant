#!/usr/bin/env bash
# Build the release zip: dist/EldenResonant-<version>.zip
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
VERSION="$(grep -m1 '^version' "$ROOT/mod/Cargo.toml" | cut -d'"' -f2)"
VGM_URL="https://github.com/vgmstream/vgmstream-releases/releases/download/nightly/vgmstream-win64.zip"
VGM_SHA="${VGM_SHA:-51dd830707225fc46706742af321b03ccd98206ca660397caec0692760c8d4e7}"

python3 "$ROOT/tools/preflight.py"
python3 "$ROOT/tools/gen.py"
(cd "$ROOT/mod" && cargo test -q && cargo build --release --target x86_64-pc-windows-gnu -q)

CACHE="$ROOT/.cache"; mkdir -p "$CACHE"
if [ ! -f "$CACHE/vgmstream-win64.zip" ]; then curl -sSfL -o "$CACHE/vgmstream-win64.zip" "$VGM_URL"; fi
echo "$VGM_SHA  $CACHE/vgmstream-win64.zip" | sha256sum -c -

STAGE="$ROOT/dist/stage"; OUT="$ROOT/dist/EldenResonant-$VERSION.zip"
rm -rf "$STAGE" "$OUT"; mkdir -p "$STAGE/EldenResonant/vgmstream" "$STAGE/EldenResonant/licenses"
cp "$ROOT/package/config_eldenring.toml" "$STAGE/"
cp "$ROOT/mod/target/x86_64-pc-windows-gnu/release/elden_resonant.dll" "$STAGE/EldenResonant/"
cp "$ROOT/package/elden-resonant.ini" "$ROOT/package/README.txt" "$STAGE/EldenResonant/"
(cd "$STAGE/EldenResonant/vgmstream" && unzip -q "$CACHE/vgmstream-win64.zip" -x "USAGE.md")
cp "$STAGE/EldenResonant/vgmstream/COPYING" "$STAGE/EldenResonant/licenses/vgmstream-COPYING.txt"
python3 "$ROOT/tools/third_party.py" > "$STAGE/EldenResonant/licenses/rust-crates.txt"
# Layout matches Melty's ModEngine2 folder: modengine2/config_eldenring.toml + modengine2/EldenResonant/
mkdir -p "$STAGE/pkg/modengine2" && mv "$STAGE/config_eldenring.toml" "$STAGE/EldenResonant" "$STAGE/pkg/modengine2/"
(cd "$STAGE/pkg" && find modengine2 -exec touch -d "2026-01-01 00:00:00" {} + && zip -qrX "$OUT" modengine2)
rm -rf "$STAGE"
echo "built $OUT"
sha256sum "$OUT"; stat -c '%s bytes' "$OUT"
