//! The prebuilt Flutter embedder engine for a target, as a table.
//!
//! Kept free of I/O so it can be tested: the build script includes it, and so
//! does `tests/engine_artifact.rs`.
//!
//! The names are Flutter's, and they are not regular. Find them with
//! `gsutil ls -r gs://flutter_infra_release/flutter/<engine_ref> | grep embedder`.

/// How the downloaded archive is laid out and linked.
#[derive(Debug, PartialEq, Eq)]
pub enum EngineLayout {
    /// `FlutterEmbedder.framework`, unzipped into a directory of that name.
    Framework,
    /// A flat zip with a header and a shared library.
    SharedLibrary {
        header: &'static str,
        library: &'static str,
        /// What `cargo:rustc-link-lib` is given.
        link_name: &'static str,
    },
}

#[derive(Debug, PartialEq, Eq)]
pub struct EngineArtifact {
    pub url: String,
    pub layout: EngineLayout,
}

const BASE: &str = "https://storage.googleapis.com/flutter_infra_release/flutter";

/// The engine for `target_os`/`target_arch`, spelled as Cargo spells them in
/// `CARGO_CFG_TARGET_OS` and `CARGO_CFG_TARGET_ARCH`.
pub fn engine_artifact(
    target_os: &str,
    target_arch: &str,
    engine_ref: &str,
) -> Result<EngineArtifact, String> {
    let arch = match target_arch {
        "x86_64" => "x64",
        "aarch64" => "arm64",
        other => {
            return Err(format!(
                "no prebuilt Flutter embedder engine is published for the {other} \
                 architecture ({target_os})"
            ))
        }
    };
    match target_os {
        // darwin-x64 is the only place FlutterEmbedder.framework is published,
        // and it is universal: x86_64 and arm64 in one binary.
        "macos" => Ok(EngineArtifact {
            url: format!("{BASE}/{engine_ref}/darwin-x64/FlutterEmbedder.framework.zip"),
            layout: EngineLayout::Framework,
        }),
        "linux" => Ok(EngineArtifact {
            url: format!("{BASE}/{engine_ref}/linux-{arch}/linux-{arch}-embedder.zip"),
            layout: EngineLayout::SharedLibrary {
                header: "flutter_embedder.h",
                library: "libflutter_engine.so",
                link_name: "flutter_engine",
            },
        }),
        // The zip holds flutter_engine.dll and its import library
        // flutter_engine.dll.lib. The MSVC linker appends `.lib` to a link
        // name, so the name to give it is the DLL's own.
        "windows" => Ok(EngineArtifact {
            url: format!("{BASE}/{engine_ref}/windows-{arch}/windows-{arch}-embedder.zip"),
            layout: EngineLayout::SharedLibrary {
                header: "flutter_embedder.h",
                library: "flutter_engine.dll",
                link_name: "flutter_engine.dll",
            },
        }),
        other => Err(format!(
            "no prebuilt Flutter embedder engine is published for {other}; the \
             terminal embedder builds on linux, macos and windows"
        )),
    }
}
