fn main() {
    if let Ok(bundled) = std::env::var("TAU_BUNDLE") {
        println!("cargo:rustc-env=TAU_BUNDLE={}", bundled);
    }
}
