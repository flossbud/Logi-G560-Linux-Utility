#!/usr/bin/env bash
# Build a portable G560 Linux Utility AppImage.
#
# Prerequisites (installed on the host or CI runner):
#   - rustc/cargo (1.97.1 via rust-toolchain)
#   - gcc, pkg-config
#   - clang, libclang-dev (bindgen needs libclang for libspa-sys)
#   - libwebkit2gtk-4.1-dev, libayatana-appindicator3-dev
#   - libgstreamer1.0-dev >= 1.24, libgstreamer-plugins-base1.0-dev,
#     libgstreamer-plugins-bad1.0-dev, gstreamer1.0-pipewire
#     (Ubuntu 24.04 ships 1.24; Ubuntu 22.04 ships 1.20, too old)
#   - libpipewire-0.3-dev, libusb-1.0-0-dev, libgtk-3-dev
#   - patchelf, desktop-file-utils, wget or curl
#
# Downloads (cached in .cache/appimage/):
#   - linuxdeploy-x86_64.AppImage        (continuous)
#   - linuxdeploy-plugin-gtk.sh          (continuous)
#   - linuxdeploy-plugin-gstreamer.sh    (continuous)
#   - appimagetool-x86_64.AppImage       (continuous)

set -euo pipefail

ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)"
cd "$ROOT"

if [[ "$(uname -m)" != "x86_64" ]]; then
    echo "error: only x86_64 is supported for now (host is $(uname -m))" >&2
    exit 1
fi

CACHE_DIR="$ROOT/.cache/appimage"
DIST_DIR="$ROOT/dist"
APPDIR="$ROOT/AppDir"
mkdir -p "$CACHE_DIR" "$DIST_DIR"

download_if_missing() {
    local url="$1" dest="$2"
    if [[ ! -x "$dest" ]]; then
        echo ">>> Downloading $(basename "$dest")"
        curl --fail --location --silent --show-error -o "$dest" "$url"
        chmod +x "$dest"
    fi
}

download_if_missing \
    "https://github.com/linuxdeploy/linuxdeploy/releases/download/continuous/linuxdeploy-x86_64.AppImage" \
    "$CACHE_DIR/linuxdeploy-x86_64.AppImage"

download_if_missing \
    "https://raw.githubusercontent.com/linuxdeploy/linuxdeploy-plugin-gtk/master/linuxdeploy-plugin-gtk.sh" \
    "$CACHE_DIR/linuxdeploy-plugin-gtk.sh"

download_if_missing \
    "https://raw.githubusercontent.com/linuxdeploy/linuxdeploy-plugin-gstreamer/master/linuxdeploy-plugin-gstreamer.sh" \
    "$CACHE_DIR/linuxdeploy-plugin-gstreamer.sh"

download_if_missing \
    "https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-x86_64.AppImage" \
    "$CACHE_DIR/appimagetool-x86_64.AppImage"

echo ">>> Building release binaries"
cargo build --release --workspace

echo ">>> Staging AppDir at $APPDIR"
rm -rf "$APPDIR"
mkdir -p \
    "$APPDIR/usr/bin" \
    "$APPDIR/usr/share/applications" \
    "$APPDIR/usr/share/icons/hicolor/32x32/apps" \
    "$APPDIR/usr/share/icons/hicolor/128x128/apps" \
    "$APPDIR/usr/share/icons/hicolor/256x256/apps" \
    "$APPDIR/usr/share/logig560"

install -m 0755 "$ROOT/target/release/logig560" "$APPDIR/usr/bin/logig560"
install -m 0755 "$ROOT/target/release/logig560-gui" "$APPDIR/usr/bin/logig560-gui"

install -m 0644 "$ROOT/contrib/70-g560.rules"                          "$APPDIR/usr/share/logig560/"
install -m 0755 "$ROOT/scripts/install-udev-rule.sh"                   "$APPDIR/usr/share/logig560/"
install -m 0644 "$ROOT/systemd/logig560-desktop.service.in"            "$APPDIR/usr/share/logig560/"
install -m 0644 "$ROOT/systemd/logig560-gaming.service.in"             "$APPDIR/usr/share/logig560/"

install -m 0644 "$ROOT/crates/logig560-gui/icons/32x32.png"            "$APPDIR/usr/share/icons/hicolor/32x32/apps/logig560.png"
install -m 0644 "$ROOT/crates/logig560-gui/icons/128x128.png"          "$APPDIR/usr/share/icons/hicolor/128x128/apps/logig560.png"
install -m 0644 "$ROOT/crates/logig560-gui/icons/128x128@2x.png"       "$APPDIR/usr/share/icons/hicolor/256x256/apps/logig560.png"
# Top-level icon (appimagetool convention).
install -m 0644 "$ROOT/crates/logig560-gui/icons/128x128@2x.png"       "$APPDIR/logig560.png"

cat > "$APPDIR/usr/share/applications/logig560.desktop" <<'DESKTOP'
[Desktop Entry]
Type=Application
Name=G560 Linux Utility
GenericName=Speaker Lighting Control
Comment=Per-zone lighting and screen-matched ambient light for Logitech G560 speakers
Exec=logig560-gui %U
Icon=logig560
Terminal=false
Categories=Utility;Settings;HardwareSettings;
StartupWMClass=logig560-gui
DESKTOP

# Also copy the .desktop to AppDir root (linuxdeploy expects it there).
install -m 0644 "$APPDIR/usr/share/applications/logig560.desktop" "$APPDIR/logig560.desktop"

echo ">>> Running linuxdeploy with gtk + gstreamer plugins"
# Skip linuxdeploy's strip pass. The bundled strip is older than Arch's
# binutils and does not understand the `.relr.dyn` section produced by
# ld with --pack-relative-relocs, which causes strip to fail on every
# Arch-sourced library. The unstripped AppImage is larger but works;
# older-glibc build hosts do not need this override to succeed, so it is
# safe to always set.
export NO_STRIP=true

# GStreamer plugin directory. linuxdeploy-plugin-gstreamer hardcodes
# /usr/lib/gstreamer-1.0, which is correct on Debian/Ubuntu but wrong on
# Fedora / RHEL / Bazzite (which use /usr/lib64). Auto-detect once here.
if [[ -d /usr/lib64/gstreamer-1.0 ]]; then
    export GSTREAMER_PLUGINS_DIR=/usr/lib64/gstreamer-1.0
elif [[ -d /usr/lib/x86_64-linux-gnu/gstreamer-1.0 ]]; then
    export GSTREAMER_PLUGINS_DIR=/usr/lib/x86_64-linux-gnu/gstreamer-1.0
fi

# Restrict the gstreamer plugin to the subset we actually load. Any plugin
# listed in GSTREAMER_INCLUDE_LIBRARIES is copied; everything else is
# skipped. Keep this in sync with the spec §1 subset list.
export GSTREAMER_INCLUDE_LIBRARIES="\
libgstpipewire.so \
libgstcoreelements.so \
libgstvideoconvertscale.so \
libgstvideofilter.so \
libgstapp.so \
libgsttypefindfunctions.so \
libgstautodetect.so"

"$CACHE_DIR/linuxdeploy-x86_64.AppImage" \
    --appdir "$APPDIR" \
    --executable "$APPDIR/usr/bin/logig560-gui" \
    --executable "$APPDIR/usr/bin/logig560" \
    --desktop-file "$APPDIR/logig560.desktop" \
    --icon-file "$APPDIR/logig560.png" \
    --plugin gtk \
    --plugin gstreamer

# WebKit spawns helper processes (WebKitNetworkProcess, WebKitWebProcess,
# WebKitGPUProcess) via a *hardcoded absolute path* baked into
# libwebkit2gtk-4.1.so at build time (LIBEXECDIR/webkit2gtk-<API>). The
# path only exists on the build system and cannot be overridden with an
# env var on Ubuntu 24.04's webkit2gtk (WEBKIT_EXEC_PATH is not compiled
# in). To make the AppImage portable we (a) bundle the helpers, (b)
# binary-patch the hardcoded path in libwebkit2gtk-4.1.so to point at a
# fixed short path, and (c) have AppRun symlink that path to the bundled
# helper directory before launching.
WEBKIT_LIBDIR=""
for candidate in \
    /usr/lib/x86_64-linux-gnu/webkit2gtk-4.1 \
    /usr/lib64/webkit2gtk-4.1 \
    /usr/lib/webkit2gtk-4.1; do
    if [[ -d "$candidate" ]]; then
        WEBKIT_LIBDIR="$candidate"
        break
    fi
done
if [[ -z "$WEBKIT_LIBDIR" ]]; then
    echo "error: cannot find webkit2gtk-4.1 helper directory on host" >&2
    exit 1
fi
echo ">>> Bundling WebKit helpers from $WEBKIT_LIBDIR"
mkdir -p "$APPDIR/usr/lib/webkit2gtk-4.1"
cp -a "$WEBKIT_LIBDIR"/. "$APPDIR/usr/lib/webkit2gtk-4.1/"
# Rewrite rpaths on the bundled helper binaries so they find bundled libs
# instead of falling back to host /usr/lib.
for bin in "$APPDIR/usr/lib/webkit2gtk-4.1"/{WebKitNetworkProcess,WebKitWebProcess,WebKitGPUProcess,MiniBrowser}; do
    [[ -f "$bin" ]] && patchelf --set-rpath '$ORIGIN/..' "$bin" || true
done

# Binary-patch libwebkit2gtk to redirect the hardcoded helper lookup path.
# We rewrite the two occurrences of the build-system path in place with a
# short null-padded replacement; AppRun then creates a symlink at the
# replacement path pointing at the bundled helper dir. The replacement
# path is fixed so it can be baked in, and short so we don't need to
# shift any file offsets.
echo ">>> Patching WebKit hardcoded helper paths"
WEBKIT_SO=""
for candidate in \
    "$APPDIR/usr/lib/libwebkit2gtk-4.1.so.0" \
    "$APPDIR/usr/lib/libwebkit2gtk-4.1.so"; do
    if [[ -f "$candidate" ]]; then
        WEBKIT_SO="$candidate"
        break
    fi
done
if [[ -z "$WEBKIT_SO" ]]; then
    echo "error: bundled libwebkit2gtk-4.1.so not found in AppDir" >&2
    exit 1
fi
python3 - "$WEBKIT_SO" <<'PY'
import sys
path = sys.argv[1]
# Original strings (build-system paths, null-terminated).
targets = [
    b'/usr/lib/x86_64-linux-gnu/webkit2gtk-4.1\x00',                    # 41 bytes
    b'/usr/lib/x86_64-linux-gnu/webkit2gtk-4.1/injected-bundle/\x00',   # 57 bytes
]
# Replacements: fixed short prefix `/tmp/.g560wk` (12 chars). AppRun
# symlinks /tmp/.g560wk -> $HERE/usr/lib/webkit2gtk-4.1 before launch.
# Pad each replacement to match the original length with trailing NULs
# so no file-offset shift is needed.
replacements = [
    b'/tmp/.g560wk\x00',                                                # helper dir
    b'/tmp/.g560wk/injected-bundle/\x00',                               # injected bundle
]
with open(path, 'rb') as f:
    data = bytearray(f.read())
for target, repl in zip(targets, replacements):
    idx = data.find(target)
    if idx < 0:
        raise SystemExit(f'error: pattern not found: {target!r}')
    padded = repl + b'\x00' * (len(target) - len(repl))
    if len(padded) != len(target):
        raise SystemExit('padding math wrong')
    data[idx:idx + len(target)] = padded
    print(f'  patched offset {idx}: {target[:-1].decode()} -> {repl[:-1].decode()}')
with open(path, 'wb') as f:
    f.write(data)
PY

echo ">>> Overwriting AppRun with our dispatcher"
cat > "$APPDIR/AppRun" <<'APPRUN'
#!/bin/sh
HERE="$(dirname "$(readlink -f "$0")")"

export LD_LIBRARY_PATH="$HERE/usr/lib:${LD_LIBRARY_PATH:-}"
export XDG_DATA_DIRS="$HERE/usr/share:${XDG_DATA_DIRS:-/usr/local/share:/usr/share}"
export GST_PLUGIN_SYSTEM_PATH="$HERE/usr/lib/gstreamer-1.0"
export GST_PLUGIN_PATH="$HERE/usr/lib/gstreamer-1.0"
export GST_PLUGIN_SCANNER="$HERE/usr/lib/gstreamer-1.0/gst-plugin-scanner"
export GIO_MODULE_DIR="$HERE/usr/lib/gio/modules"
export GDK_PIXBUF_MODULE_FILE="$HERE/usr/lib/gdk-pixbuf-2.0/loaders.cache"
export WEBKIT_DISABLE_COMPOSITING_MODE=1
export GTK_USE_PORTAL=1
# libwebkit2gtk was binary-patched to look for helpers at /tmp/.g560wk.
# Create a symlink there pointing at the bundled dir. Best-effort: if
# another instance already put one there for a different AppImage it
# gets replaced; if two AppImages coexist they'll clobber each other's
# symlink, but for a single install of this app it is stable.
ln -sfn "$HERE/usr/lib/webkit2gtk-4.1" /tmp/.g560wk 2>/dev/null || true

# Flag dispatch (primary): --cli routes into the CLI binary.
if [ "${1:-}" = "--cli" ]; then
    shift
    exec "$HERE/usr/bin/logig560" "$@"
fi

# argv[0] convenience: a `logig560` symlink runs the CLI.
case "$(basename "$0")" in
    logig560) exec "$HERE/usr/bin/logig560" "$@" ;;
esac

exec "$HERE/usr/bin/logig560-gui" "$@"
APPRUN
chmod +x "$APPDIR/AppRun"

VERSION="$(sed -n 's/^version[[:space:]]*=[[:space:]]*"\([^"]*\)"/\1/p' "$ROOT/Cargo.toml" | head -n1)"
OUTPUT="$DIST_DIR/G560-Linux-Utility-${VERSION}-x86_64.AppImage"

echo ">>> Packaging with appimagetool"
"$CACHE_DIR/appimagetool-x86_64.AppImage" --no-appstream "$APPDIR" "$OUTPUT"

SIZE_MB=$(du -m "$OUTPUT" | cut -f1)
SHA=$(sha256sum "$OUTPUT" | cut -d' ' -f1)
echo
echo ">>> Built: $OUTPUT"
echo "    size:   ${SIZE_MB} MiB"
echo "    sha256: $SHA"

if (( SIZE_MB > 250 )); then
    echo "error: AppImage exceeds 250 MiB budget (${SIZE_MB} MiB); review bundled plugins" >&2
    exit 1
fi
