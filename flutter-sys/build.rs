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
//!
//! Which engine is chosen from the *target*, read from `CARGO_CFG_TARGET_OS`
//! and `CARGO_CFG_TARGET_ARCH`. It used to be `cfg!(target_os)`, which in a
//! build script is the host the script runs on: every host but macOS fetched
//! the linux-x64 engine, Windows included.

extern crate bindgen;

#[path = "build/engine_artifact.rs"]
mod engine_artifact;

use engine_artifact::{engine_artifact, EngineLayout};
use std::env;
use std::fs;
use std::path::Path;
use std::process::Command;

fn main() {
    let engine_ref_path = Path::new("../third_party/flutter/bin/internal/engine.version");

    // Rerun this script when these files change.
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=build/engine_artifact.rs");
    println!(
        "cargo:rerun-if-changed={}",
        engine_ref_path.to_str().unwrap()
    );

    println!("cargo:rerun-if-env-changed=FLT_ENGINE_REVISION");

    let engine_ref = engine_revision(engine_ref_path);
    let engine_ref = engine_ref.trim();

    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap();
    let target_arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap();
    let artifact = engine_artifact(&target_os, &target_arch, engine_ref)
        .unwrap_or_else(|reason| panic!("{reason}"));

    let out_dir_env = env::var("OUT_DIR").unwrap();
    let out_dir = Path::new(&out_dir_env);

    let downloaded_file = artifact.url.split('/').last().unwrap();
    let embedder_zip_path = out_dir.join(downloaded_file);

    // Download the zip file containing the Flutter engine dynamic library.
    //
    // --fail, because without it a 404 writes the error page to the output
    // file and exits 0, and the failure surfaces later as an unzip error about
    // a file that is not a zip.
    assert!(
        Command::new("curl")
            .args(["--fail", "--location", "--silent", "--show-error"])
            .arg(&artifact.url)
            .arg("--output")
            .arg(&embedder_zip_path)
            .status()
            .unwrap()
            .success(),
        "could not download {}",
        artifact.url
    );

    let flutter_embedder_header_path = match artifact.layout {
        EngineLayout::Framework => {
            let framework_dir = out_dir.join("FlutterEmbedder.framework");
            unzip(&embedder_zip_path, &framework_dir);
            framework_dir.join("Headers").join("FlutterEmbedder.h")
        }
        EngineLayout::SharedLibrary { header, .. } => {
            unzip(&embedder_zip_path, out_dir);
            out_dir.join(header)
        }
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
    match artifact.layout {
        EngineLayout::Framework => {
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
        }
        EngineLayout::SharedLibrary {
            library, link_name, ..
        } => {
            // Matches `libflutter_engine.so`, or `flutter_engine.dll.lib`.
            println!("cargo:rustc-link-lib=dylib={link_name}");
            println!("cargo:rustc-link-search=native={}", out_dir.to_str().unwrap());
            println!("cargo:flutter_engine_library={library}");
        }
    }

    // Passed to the dependent binary crate to set the runtime search paths.
    println!(
        "cargo:flutter_engine_lib_path={}",
        out_dir.to_str().unwrap()
    );
}

fn unzip(src: &Path, dest: &Path) {
    fs::create_dir_all(dest).unwrap();
    // Windows has no unzip, and has had bsdtar as tar.exe since 1803; bsdtar
    // reads zip archives.
    if cfg!(windows) {
        assert!(Command::new("tar")
            .arg("-xf")
            .arg(src)
            .arg("-C")
            .arg(dest)
            .status()
            .unwrap()
            .success());
        return;
    }
    let run = || {
        Command::new("unzip")
            // Overwrite, quietly: the framework is ~90 MB of listing otherwise.
            .args(["-o", "-q"])
            .arg(src)
            .arg("-d")
            .arg(dest)
            .status()
            .unwrap()
            .success()
    };
    // For some reason on macOS, the first extraction can fail and doing it
    // again always works (upstream's note; suspected antivirus).
    assert!(run() || run(), "could not unzip {}", src.display());
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
    let output = flutter_command()
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

/// `flutter`, runnable on this host.
///
/// On Windows it is `flutter.bat`, and `Command::new("flutter")` looks only for
/// `flutter.exe`, so it is run through `cmd /C`, which resolves batch files
/// the way a shell does.
fn flutter_command() -> Command {
    if cfg!(windows) {
        let mut command = Command::new("cmd");
        command.args(["/C", "flutter"]);
        command
    } else {
        Command::new("flutter")
    }
}
