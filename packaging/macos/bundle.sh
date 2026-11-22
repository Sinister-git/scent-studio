#!/bin/sh
# Run from the repository root on macOS.
set -eu
cargo build --release --locked
app='target/release/Scent Studio.app'
mkdir -p "$app/Contents/MacOS"
install -m 755 target/release/scent-studio "$app/Contents/MacOS/scent-studio"
cp packaging/macos/Info.plist "$app/Contents/Info.plist"
codesign --force --sign - "$app"
printf 'Built %s\n' "$app"
