/// build.rs — locates and links libheif.
///
/// On macOS with Homebrew the library lives under the Homebrew prefix
/// (e.g. /opt/homebrew on Apple Silicon, /usr/local on Intel).
use std::env;
use std::fs;
use std::path::PathBuf;

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

    #[cfg(target_os = "windows")]
    {
        println!("cargo:rustc-link-lib=advapi32");
    }

    println!("cargo:rerun-if-changed=build.rs");

    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR is not set"));

    let output = out_dir.join("gray_gamma_22.icc");

    // Standard D50 media white point used by ICC profiles.
    let white_point = lcms2::CIExyY::d50();
    // Standard D50 media white point used by ICC profiles.
    let curve = lcms2::ToneCurve::new(2.2);

    let profile = lcms2::Profile::new_gray(white_point, &curve)
        .expect("failed to create Gray Gamma 2.2 ICC profile");

    assert_eq!(
        profile.color_space(),
        lcms2::ColorSpaceSignature::GrayData,
        "generated ICC profile is not GRAY"
    );

    let estimated_gamma = curve
        .estimated_gamma(0.01)
        .expect("could not estimate ICC gamma");

    assert!(
        (estimated_gamma - 2.2).abs() < 0.05,
        "generated ICC profile has unexpected gamma: {estimated_gamma}"
    );

    let icc = profile
        .icc()
        .expect("failed to serialize Gray Gamma 2.2 ICC profile");

    assert!(!icc.is_empty(), "generated ICC profile is empty");

    // Verify that the serialized bytes can be parsed back.
    let parsed = lcms2::Profile::new_icc(&icc).expect("generated ICC bytes cannot be parsed");
    assert_eq!(
        parsed.color_space(),
        lcms2::ColorSpaceSignature::GrayData,
        "serialized ICC profile is not GRAY"
    );

    fs::write(&output, &icc).expect("failed to write Gray Gamma 2.2 ICC profile");

    println!(
        "cargo:warning=Generated Gray Gamma 2.2 ICC profile: {} bytes",
        icc.len()
    );
}
