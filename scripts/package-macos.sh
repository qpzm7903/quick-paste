#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

ARCH=""
SKIP_BUILD=0

while [[ $# -gt 0 ]]; do
    case "$1" in
        --arch)
            ARCH="${2:-}"
            shift 2
            ;;
        --skip-build)
            SKIP_BUILD=1
            shift
            ;;
        -h|--help)
            echo "用法: $0 [--arch arm64|x64|universal] [--skip-build]"
            exit 0
            ;;
        *)
            echo "未知参数: $1" >&2
            exit 1
            ;;
    esac
done

if [[ -z "$ARCH" ]]; then
    HOST_ARCH="$(uname -m)"
    if [[ "$HOST_ARCH" == "arm64" ]]; then
        ARCH="arm64"
    else
        ARCH="x64"
    fi
fi

cd "$PROJECT_ROOT"

VERSION="$(python3 -c "import tomllib; print(tomllib.load(open('Cargo.toml','rb'))['package']['version'])")"
HOST_TARGET="$(rustc -vV | sed -n 's/^host: //p')"

resolve_binary() {
    local rust_target="$1"
    local target_bin="$PROJECT_ROOT/target/$rust_target/release/quick-paste"
    local host_bin="$PROJECT_ROOT/target/release/quick-paste"
    if [[ -f "$target_bin" ]]; then
        echo "$target_bin"
    elif [[ "$rust_target" == "$HOST_TARGET" && -f "$host_bin" ]]; then
        echo "$host_bin"
    else
        return 1
    fi
}

build_target() {
    local rust_target="$1"
    if [[ "$SKIP_BUILD" -eq 0 ]]; then
        echo "🔨 构建 macOS ($rust_target)..."
        cargo build --release --locked --target "$rust_target"
    fi
}

case "$ARCH" in
    arm64)
        RUST_TARGET="aarch64-apple-darwin"
        OUT_SUBDIR="aarch64-apple-darwin"
        build_target "$RUST_TARGET"
        BIN_PATH="$(resolve_binary "$RUST_TARGET")" || {
            echo "❌ 找不到编译产物（尝试 $RUST_TARGET/release 或 release）" >&2
            exit 1
        }
        ;;
    x64)
        RUST_TARGET="x86_64-apple-darwin"
        OUT_SUBDIR="x86_64-apple-darwin"
        build_target "$RUST_TARGET"
        BIN_PATH="$(resolve_binary "$RUST_TARGET")" || {
            echo "❌ 找不到编译产物（尝试 $RUST_TARGET/release 或 release）" >&2
            exit 1
        }
        ;;
    universal)
        OUT_SUBDIR="universal-apple-darwin"
        build_target "aarch64-apple-darwin"
        build_target "x86_64-apple-darwin"
        ARM_BIN="$(resolve_binary "aarch64-apple-darwin")"
        X64_BIN="$(resolve_binary "x86_64-apple-darwin")"
        BIN_PATH="$PROJECT_ROOT/target/$OUT_SUBDIR/release/quick-paste"
        mkdir -p "$(dirname "$BIN_PATH")"
        lipo -create "$ARM_BIN" "$X64_BIN" -output "$BIN_PATH"
        ;;
    *)
        echo "❌ 不支持的架构: $ARCH（支持 arm64 | x64 | universal）" >&2
        exit 1
        ;;
esac

OUT_DIR="$PROJECT_ROOT/target/packages/$OUT_SUBDIR"
APP_BUNDLE="$OUT_DIR/QuickPaste.app"
DMG_PATH="$OUT_DIR/quick-paste_${VERSION}_${ARCH}.dmg"
ZIP_PATH="$OUT_DIR/quick-paste_${VERSION}_${ARCH}-app.zip"

rm -rf "$APP_BUNDLE" "$DMG_PATH" "$ZIP_PATH"
mkdir -p "$APP_BUNDLE/Contents/MacOS" "$APP_BUNDLE/Contents/Resources"

cp "$BIN_PATH" "$APP_BUNDLE/Contents/MacOS/quick-paste"
chmod +x "$APP_BUNDLE/Contents/MacOS/quick-paste"

python3 "$SCRIPT_DIR/generate-icon.py" "$APP_BUNDLE/Contents/Resources/QuickPaste.icns"

sed "s/{{VERSION}}/$VERSION/g" "$PROJECT_ROOT/assets/macos/Info.plist.template" > "$APP_BUNDLE/Contents/Info.plist"

echo "🔏 执行 Ad-hoc 代码签名..."
codesign --force --deep --sign - "$APP_BUNDLE"
codesign --verify --deep --strict "$APP_BUNDLE"

echo "📦 生成便携 ZIP: $ZIP_PATH"
ditto -c -k --sequesterRsrc --keepParent "$APP_BUNDLE" "$ZIP_PATH"

echo "💿 生成 DMG 安装镜像: $DMG_PATH"
STAGE_DIR="$(mktemp -d "$OUT_DIR/.dmg-stage.XXXXXX")"
cleanup() {
    rm -rf "$STAGE_DIR"
}
trap cleanup EXIT

cp -R "$APP_BUNDLE" "$STAGE_DIR/QuickPaste.app"
ln -s /Applications "$STAGE_DIR/Applications"
hdiutil create \
    -volname "QuickPaste $VERSION" \
    -srcfolder "$STAGE_DIR" \
    -ov \
    -format UDZO \
    "$DMG_PATH" >/dev/null

if [[ ! -s "$DMG_PATH" || ! -s "$ZIP_PATH" ]]; then
    echo "❌ macOS 打包产物为空" >&2
    exit 1
fi

echo "✅ macOS 打包完成："
echo "   - App: $APP_BUNDLE"
echo "   - DMG: $DMG_PATH"
echo "   - ZIP: $ZIP_PATH"
