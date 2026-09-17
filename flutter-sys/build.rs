//! This build script links the output binary with the Flutter embedder dynamic
//! library.
//!
//! Because building the shared library from source is too complicated, this
//! script simply downloads the headers and prebuilt Flutter dynamic library,
//! runs `bindgen` to generate Rust bindings from these headers, and then links
//! against the library.
//!
//! The version of the binaries will correspond to the same git commit ref as
//! the same file located in the
//! `third_party/flutter/bin/internal/engine.version` submodule.

extern crate bindgen;

use std::env;
use std::fs;
use std::path::Path;
use std::process::Command;

// TODO(jiahaog): Rewrite this into separate scripts for macOS and Linux.
fn main() {
    let engine_ref_path = Path::new("../third_party/flutter/bin/internal/engine.version");

    // Rerun this script when these files change.
    println!("cargo:rerun-if-changed=build.rs");
    println!(
        "cargo:rerun-if-changed={}",
        engine_ref_path.to_str().unwrap()
    );

    println!("cargo:rerun-if-env-changed=FLT_ENGINE_REVISION");

    let engine_ref = engine_revision(engine_ref_path);
    let engine_ref = engine_ref.trim();

    let out_dir_env = env::var("OUT_DIR").unwrap();
    let out_dir = Path::new(&out_dir_env);

    let engine_url = engine_url(engine_ref);
    let downloaded_file = engine_url.split('/').last().unwrap();

    let embedder_zip_path = out_dir.join(downloaded_file);

    // Download the zip file containing the Flutter engine dynamic library.
    assert!(Command::new("curl")
        .arg(engine_url)
        .arg("--output")
        .arg(embedder_zip_path.clone())
        .status()
        .unwrap()
        .success());

    if cfg!(target_os = "macos") {
        let framework_dir = out_dir.join("FlutterEmbedder.framework");
        unzip(&embedder_zip_path, &framework_dir);
    } else {
        unzip(&embedder_zip_path, out_dir);
    };

    // There will be two files of interest in the unzipped output:
    // (On Linux):
    // - The headers: flutter_embedder.h for bindgen.
    // - The dynamic library: libflutter_engine.so for linking.

    let flutter_embedder_header_path = if cfg!(target_os = "macos") {
        out_dir
            .join("FlutterEmbedder.framework")
            .join("Headers")
            .join("FlutterEmbedder.h")
    } else {
        out_dir.join("flutter_embedder.h")
    };
    let flutter_embedder_header_path = flutter_embedder_header_path.to_str().unwrap();

    let bindings = bindgen::Builder::default()
        .header(flutter_embedder_header_path)
        .parse_callbacks(Box::new(bindgen::CargoCallbacks))
        .generate()
        .expect("Unable to generate bindings");

    bindings
        .write_to_file(out_dir.join("bindings.rs"))
        .expect("Couldn't write bindings!");

    // Link against the Flutter shared library.
    if cfg!(target_os = "macos") {
        // On macOS, ld will link using `-l${rustc-link-lib}` which looks for
        // `lib${rustc-link-lib}.dylib.
        //
        // Matches `libFlutterEmbedder.dylib`.
        println!("cargo:rustc-link-lib=framework=FlutterEmbedder");
        println!(
            "cargo:rustc-link-search=framework={}",
            out_dir.to_str().unwrap()
        );
        // Needed for `cargo test`.
        println!(
            "cargo:rustc-link-arg=-Wl,-rpath,{}",
            out_dir.to_str().unwrap()
        );
    } else {
        // Matches `libflutter_engine.so`.
        println!("cargo:rustc-link-lib=flutter_engine");
        println!("cargo:rustc-link-search={}", out_dir.to_str().unwrap());
    };

    // Passed to the dependent binary crate to set the runtime search paths.
    println!(
        "cargo:flutter_engine_lib_path={}",
        out_dir.to_str().unwrap()
    );
}

fn engine_url(engine_ref: &str) -> String {
    // This is tricky to figure out and can change between releases.
    //
    // Use the following to find it:
    // ```
    // gsutil ls -r gs://flutter_infra_release/flutter/{engine_ref} | grep embedder
    // ```
    // Source: https://www.industrialflutter.com/blogs/where-to-find-prebuilt-flutter-engine-artifacts/
    if cfg!(target_os = "macos") {
        format!("https://storage.googleapis.com/flutter_infra_release/flutter/{engine_ref}/darwin-x64/FlutterEmbedder.framework.zip")
    } else {
        format!("https://storage.googleapis.com/flutter_infra_release/flutter/{engine_ref}/linux-x64/linux-x64-embedder.zip")
    }
}

fn unzip(src: &Path, dest: &Path) {
    assert!(Command::new("unzip")
        // Overwrite.
        .arg("-o")
        .arg(src)
        .arg("-d")
        .arg(dest)
        .status()
        .unwrap()
        .success());

    // For some reason on macOS, the above command will fail to extract the zip file?
    // And doing it again always works.
    //
    // ```
    // $ "unzip" "-o" "/Users/jiahaog/dev/flt/target/debug/build/flutter-sys-411194cdfb6611b7/out/FlutterEmbedder.framework.zip" "-d" "/Users/jiahaog/dev/flt/target/debug/build/flutter-sys-411194cdfb6611b7/out"
    // Archive:  /Users/jiahaog/dev/flt/target/debug/build/flutter-sys-411194cdfb6611b7/out/FlutterEmbedder.framework.zip
    // inflating: /Users/jiahaog/dev/flt/target/debug/build/flutter-sys-411194cdfb6611b7/out/FlutterEmbedder.framework.zip
    // ```
    // TODO(jiahaog): Figure this out, I suspect antivirus.
    if cfg!(target_os = "macos") {
        assert!(Command::new("unzip")
            // Overwrite.
            .arg("-o")
            .arg(src)
            .arg("-d")
            .arg(dest)
            .status()
            .unwrap()
            .success());
    }
}

/// The engine revision to link against.
///
/// `FLT_ENGINE_REVISION` first, then the Flutter submodule's
/// `engine.version`, which is upstream's arrangement. Without either -- a
/// shallow clone with no submodules, which is how Dartvel installs this --
/// the revision of the `flutter` on PATH, because that Flutter builds the
/// bundle this engine will run, and a kernel from one engine does not load in
/// another.
fn engine_revision(file: &Path) -> String {
    if let Ok(revision) = env::var("FLT_ENGINE_REVISION") {
        if !revision.trim().is_empty() {
            return revision;
        }
    }
    if let Ok(revision) = fs::read_to_string(file) {
        return revision;
    }
    let output = Command::new("flutter")
        .args(["--version", "--machine"])
        .output()
        .unwrap_or_else(|e| {
            panic!(
                "{} is missing and `flutter --version --machine` could not run ({e}). \
                 Set FLT_ENGINE_REVISION or put flutter on PATH.",
                file.display()
            )
        });
    let text = String::from_utf8_lossy(&output.stdout);
    engine_revision_from_machine_version(&text).unwrap_or_else(|| {
        panic!(
            "{} is missing and `flutter --version --machine` named no engineRevision:\n{text}",
            file.display()
        )
    })
}

/// `engineRevision` out of `flutter --version --machine`, without a JSON
/// dependency in a build script.
fn engine_revision_from_machine_version(text: &str) -> Option<String> {
    let key = text.find("\"engineRevision\"")?;
    let rest = &text[key + "\"engineRevision\"".len()..];
    let start = rest.find('"')? + 1;
    let end = start + rest[start..].find('"')?;
    let revision = &rest[start..end];
    if revision.len() == 40 && revision.chars().all(|c| c.is_ascii_hexdigit()) {
        Some(revision.to_string())
    } else {
        None
    }
}
