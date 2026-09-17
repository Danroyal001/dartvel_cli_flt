fn main() {
    let flutter_engine_lib_path = std::env::var(
        // `DEP_${Cargo.toml.links of flutter-sys crate}_${actual envar pair in build.rs}`.
        "DEP_FLUTTER_ENGINE_FLUTTER_ENGINE_LIB_PATH",
    )
    .unwrap();

    // The target, not `cfg!(target_os)`: in a build script that is the host.
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap();

    match target_os.as_str() {
        "macos" => {
            // With SIP there is no environment variable that overrides where a
            // framework is loaded from, so the search path is in the binary.
            //
            // `@executable_path/lib` first: that is where `dartvel-cli-flt
            // build` puts FlutterEmbedder.framework, and a shipped bundle must
            // load its own engine rather than whatever this machine's cargo
            // output directory held. The output directory after it keeps
            // `cargo run` working.
            println!("cargo:rustc-link-arg=-Wl,-rpath,@executable_path/lib");
            println!("cargo:rustc-link-arg=-Wl,-rpath,{flutter_engine_lib_path}");
        }
        "linux" => {
            // `$ORIGIN/lib`, for the same reason as macOS: the bundle's engine
            // is beside the binary. The launcher also sets LD_LIBRARY_PATH,
            // which keeps working for bundles assembled before this.
            println!("cargo:rustc-link-arg=-Wl,-rpath,$ORIGIN/lib");
            // Runtime environment for `cargo run` only.
            println!("cargo:rustc-env=LD_LIBRARY_PATH={flutter_engine_lib_path}");
        }
        // Windows looks for a DLL beside the executable before anywhere else,
        // so flutter_engine.dll next to flt.exe needs no search path at all.
        _ => {}
    }
}
