# RustyZoo iPad prototype

Open `RustyZoo-iPad.xcodeproj` in Xcode. Choose the RustyZoo scheme and an iPad simulator or a paired iPad. For a physical iPad, select your development team in Signing & Capabilities.

This build uses the original game panels in landscape. Tap to click, drag with one finger to use a tool, hold and release for right-click, drag with two fingers to pan, and pinch to zoom. Bring both fingers down before dragging to start a camera gesture.

The Xcode project includes the local game data when the `Zoo-Tycoon-Complete-Collection` folder is beside the RustyZoo source folder. You can also copy the folder containing `ZOO.INI` into **On My iPad > RustyZoo** in Files. The prototype uses SDL rendering and saves in its own Documents folder.

For a separate build environment, install Rust 1.99.0 with its `aarch64-apple-ios` and `aarch64-apple-ios-sim` targets, then use Xcode or run `ios/package.sh iphoneos /absolute/path/RustyZoo.app` from the source checkout. The device bundle produced by the packaging script requires development signing before installation.
