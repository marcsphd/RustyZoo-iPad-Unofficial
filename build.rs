fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") {
        return;
    }
    println!("cargo:rerun-if-changed=src/window/metalfx.m");
    println!("cargo:rerun-if-changed=src/window/metalfx_motion.inc");
    cc::Build::new()
        .file("src/window/metalfx.m")
        .flag("-fobjc-arc")
        .compile("rustyzoo_metalfx");
    println!("cargo:rustc-link-lib=framework=Foundation");
    println!("cargo:rustc-link-lib=framework=Metal");
    println!("cargo:rustc-link-lib=framework=QuartzCore");
    println!("cargo:rustc-link-arg=-Wl,-weak_framework,MetalFX");
}
