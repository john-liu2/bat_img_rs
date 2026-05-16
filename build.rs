/// build.rs — locates and links libheif.
///
/// On macOS with Homebrew the library lives under the Homebrew prefix
/// (e.g. /opt/homebrew on Apple Silicon, /usr/local on Intel).
/// `pkg-config` is the primary discovery mechanism; we fall back to
/// hard-coded Homebrew paths when pkg-config is absent.

fn main() {
    // Let libheif-rs / its build script handle the linking through pkg-config.
    // We only add the Homebrew search path as a fallback for macOS.
    #[cfg(target_os = "macos")]
    {
        // Apple Silicon
        println!("cargo:rustc-link-search=native=/opt/homebrew/lib");
        // Intel Mac
        println!("cargo:rustc-link-search=native=/usr/local/lib");

        // pkg-config picks up the exact flags; these are the typical names.
        println!("cargo:rustc-link-lib=heif");
    }
}
