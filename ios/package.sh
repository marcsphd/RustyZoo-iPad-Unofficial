#!/bin/bash
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
platform="$1"
app="$2"
mkdir -p "$app"
"$here/build-executable.sh" "$platform" "$app/RustyZoo"
cp "$here/Info.plist" "$app/Info.plist"
case "$platform" in
    iphoneos) supported=iPhoneOS ;;
    iphonesimulator) supported=iPhoneSimulator ;;
esac
/usr/libexec/PlistBuddy -c "Add :CFBundleSupportedPlatforms array" "$app/Info.plist"
/usr/libexec/PlistBuddy -c "Add :CFBundleSupportedPlatforms:0 string $supported" "$app/Info.plist"
plutil -lint "$app/Info.plist"
if [[ "$platform" == iphonesimulator ]]; then
    codesign --force --sign - "$app"
fi
