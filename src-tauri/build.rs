fn main() {
    // The computer-use macOS provider references Accessibility statics
    // (kAXRoleAttribute, …) that live in the ApplicationServices umbrella
    // framework. Link it explicitly so the symbols resolve in all targets,
    // including the bare `--lib` test binary (which the Tauri app link does
    // not cover on its own).
    #[cfg(target_os = "macos")]
    println!("cargo:rustc-link-lib=framework=ApplicationServices");
    #[cfg(target_os = "macos")]
    println!("cargo:rustc-link-lib=framework=ScreenCaptureKit");

    tauri_build::build()
}
