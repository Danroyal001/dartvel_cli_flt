//! Which prebuilt engine the build script fetches, and how it links.
//!
//! The build script used to choose between two answers, macOS and "Linux", by
//! asking `cfg!(target_os)` -- which in a build script is the *host* the
//! script runs on, not the target. Windows got the Linux engine, and so did an
//! arm64 Linux machine. These tests pin the table rather than the download.
#[path = "../build/engine_artifact.rs"]
mod engine_artifact;

use engine_artifact::{engine_artifact, EngineLayout};

const REV: &str = "83675ed27633283e7fc296c8bca22e841224c096";
const BASE: &str = "https://storage.googleapis.com/flutter_infra_release/flutter";

#[test]
fn linux_x64_links_the_linux_x64_embedder() {
    let artifact = engine_artifact("linux", "x86_64", REV).unwrap();
    assert_eq!(artifact.url, format!("{BASE}/{REV}/linux-x64/linux-x64-embedder.zip"));
    assert_eq!(
        artifact.layout,
        EngineLayout::SharedLibrary {
            header: "flutter_embedder.h",
            library: "libflutter_engine.so",
            link_name: "flutter_engine",
        }
    );
}

#[test]
fn linux_arm64_does_not_get_the_x64_engine() {
    let artifact = engine_artifact("linux", "aarch64", REV).unwrap();
    assert_eq!(
        artifact.url,
        format!("{BASE}/{REV}/linux-arm64/linux-arm64-embedder.zip")
    );
}

#[test]
fn windows_x64_links_flutter_engine_dll_through_its_import_library() {
    let artifact = engine_artifact("windows", "x86_64", REV).unwrap();
    assert_eq!(
        artifact.url,
        format!("{BASE}/{REV}/windows-x64/windows-x64-embedder.zip")
    );
    // The zip carries flutter_engine.dll and flutter_engine.dll.lib. MSVC
    // appends `.lib` to the link name, so the name is the DLL's own.
    assert_eq!(
        artifact.layout,
        EngineLayout::SharedLibrary {
            header: "flutter_embedder.h",
            library: "flutter_engine.dll",
            link_name: "flutter_engine.dll",
        }
    );
}

#[test]
fn windows_arm64_gets_the_arm64_engine() {
    let artifact = engine_artifact("windows", "aarch64", REV).unwrap();
    assert_eq!(
        artifact.url,
        format!("{BASE}/{REV}/windows-arm64/windows-arm64-embedder.zip")
    );
}

#[test]
fn both_mac_architectures_use_the_universal_framework() {
    // Only darwin-x64 publishes FlutterEmbedder.framework, and it is a
    // universal binary (x86_64 and arm64); darwin-arm64/ has none and 404s.
    for arch in ["x86_64", "aarch64"] {
        let artifact = engine_artifact("macos", arch, REV).unwrap();
        assert_eq!(
            artifact.url,
            format!("{BASE}/{REV}/darwin-x64/FlutterEmbedder.framework.zip"),
            "{arch}"
        );
        assert_eq!(artifact.layout, EngineLayout::Framework, "{arch}");
    }
}

#[test]
fn an_unsupported_target_is_refused_by_name() {
    let error = engine_artifact("freebsd", "x86_64", REV).unwrap_err();
    assert!(error.contains("freebsd"), "{error}");
    let error = engine_artifact("windows", "x86", REV).unwrap_err();
    assert!(error.contains("x86"), "{error}");
}
