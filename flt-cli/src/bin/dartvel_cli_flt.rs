//! `dartvel-cli-flt` — producing a terminal application someone can ship.
//!
//! This is the half upstream does not have, and the reason Dartvel forks this
//! repository. `flt-cli` is a development loop: it builds the Flutter project,
//! compiles the embedder from source, and runs it, all against a checkout. It
//! never emits an artifact that can leave the machine.
//!
//! Dartvel needs one. `dartvel build linux-cli` promises a binary that renders
//! in a terminal and contains no GUI backend, and its preflight looks for this
//! executable by name at
//! `~/.dartvel/toolchains/dartvel_cli_flt/bin/dartvel-cli-flt`.
//!
//! What it does *not* do yet is AOT, and it says so rather than quietly
//! producing a JIT bundle when asked for a release build. Upstream notes the
//! Flutter project is always built in debug mode, and a `--release` that
//! silently shipped a debug bundle would be exactly the kind of confident wrong
//! answer this project keeps finding.
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{exit, Command};

use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(name = "dartvel-cli-flt", author, version, about)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Assemble a distributable terminal application.
    Build {
        /// Host platform being built for. Accepted for symmetry with
        /// `dartvel build <platform>-cli`; only the host is supported.
        platform: Option<String>,

        /// Path to the Flutter project. Defaults to the working directory.
        #[clap(long)]
        project: Option<String>,

        /// Where to write the bundle.
        #[clap(long, default_value = "build/terminal")]
        out: String,

        /// Build in release mode. Not supported yet — see the module comment.
        #[clap(long, default_value_t = false)]
        release: bool,
    },
}

fn main() {
    let cli = Cli::parse();
    match cli.command {
        Commands::Build {
            platform,
            project,
            out,
            release,
        } => build(platform, project, out, release),
    }
}

fn build(platform: Option<String>, project: Option<String>, out: String, release: bool) {
    if release {
        eprintln!(
            "dartvel-cli-flt: release builds are not supported yet.\n\
             \n\
             The embedder runs the Flutter project in debug (JIT) mode; AOT is\n\
             not implemented. Emitting a debug bundle in answer to --release\n\
             would ship something slower and larger than asked for, under a\n\
             name that says otherwise, so this refuses instead.\n\
             \n\
             Build without --release to produce a JIT bundle."
        );
        exit(2);
    }

    if let Some(platform) = platform.as_deref() {
        let host = if cfg!(target_os = "macos") {
            "macos"
        } else if cfg!(target_os = "windows") {
            "windows"
        } else {
            "linux"
        };
        if platform != host {
            eprintln!(
                "dartvel-cli-flt: cannot build for {platform} on {host}. The embedder \
                 links a host engine; cross-building is not supported."
            );
            exit(2);
        }
    }

    let project_dir = project
        .map(PathBuf::from)
        .unwrap_or_else(|| env::current_dir().expect("no working directory"));
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("flt-cli has a parent")
        .to_path_buf();

    // The app first: assets and the kernel the embedder will run.
    //
    // --target-platform is not optional. Without it `flutter build bundle`
    // targets Android and stops on a missing Android SDK, which is a confusing
    // thing to be told while building a terminal application for the host.
    let target_platform = if cfg!(target_arch = "aarch64") {
        "linux-arm64"
    } else {
        "linux-x64"
    };
    run(
        Command::new("flutter")
            .current_dir(&project_dir)
            .args(["build", "bundle", "--target-platform", target_platform]),
        "flutter build bundle",
    );

    // Then the embedder, optimised — it is the binary being shipped.
    run(
        Command::new("cargo")
            .current_dir(&workspace)
            .args(["build", "--release", "-p", "flt"]),
        "cargo build -p flt",
    );

    let engine = find_file(&workspace.join("target"), "libflutter_engine.so")
        .unwrap_or_else(|| {
            eprintln!(
                "dartvel-cli-flt: libflutter_engine.so was not found under {}.\n\
                 It is downloaded by flutter-sys during the build; a missing \
                 one means that build did not run.",
                workspace.join("target").display()
            );
            exit(1);
        });

    let out_dir = project_dir.join(&out);
    let lib_dir = out_dir.join("lib");
    let data_dir = out_dir.join("data");
    fs::create_dir_all(&lib_dir).expect("create lib dir");
    fs::create_dir_all(&data_dir).expect("create data dir");

    copy(&workspace.join("target/release/flt"), &out_dir.join("flt"));
    copy(&engine, &lib_dir.join("libflutter_engine.so"));

    let assets = project_dir.join("build/flutter_assets");
    if !assets.is_dir() {
        eprintln!(
            "dartvel-cli-flt: {} is missing; `flutter build bundle` did not produce \
             an asset bundle.",
            assets.display()
        );
        exit(1);
    }
    copy_dir(&assets, &data_dir.join("flutter_assets"));

    // icudtl.dat, which the engine needs and nothing in the app build emits.
    // It lives in the Flutter SDK's artifact cache, so the SDK has to be
    // locatable — the same SDK that just built the bundle.
    match find_icu_data() {
        Some(icu) => copy(&icu, &data_dir.join("icudtl.dat")),
        None => {
            eprintln!(
                "dartvel-cli-flt: icudtl.dat was not found in the Flutter SDK cache.\n\
                 The engine cannot start without it, so the bundle would be \
                 assembled and dead."
            );
            exit(1);
        }
    }

    // A launcher, because the engine is beside the binary rather than on the
    // library path. Without it the bundle only runs from a shell that already
    // knows this, which is not what "distributable" means.
    //
    // It also sends the application's own output to a log file. Without that,
    // anything the app prints is written straight onto the surface the
    // embedder is drawing on: a screenshot of a running bundle showed the
    // rendered frame shredded by interleaved `print` lines. The terminal is
    // the display here, so it cannot also be the console.
    let launcher = out_dir.join("run.sh");
    fs::write(
        &launcher,
        "#!/bin/sh\n\
         # Runs the terminal application from this directory.\n\
         here=$(cd \"$(dirname \"$0\")\" && pwd)\n\
         LD_LIBRARY_PATH=\"$here/lib:$LD_LIBRARY_PATH\" \\\n\
         exec \"$here/flt\" \\\n\
           --assets-dir \"$here/data/flutter_assets\" \\\n\
           --icu-data-path \"$here/data/icudtl.dat\" \\\n\
           --log-file \"${FLT_LOG_FILE:-$here/app.log}\" \"$@\"\n",
    )
    .expect("write launcher");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&launcher, fs::Permissions::from_mode(0o755))
            .expect("chmod launcher");
    }

    println!("dartvel-cli-flt: wrote {}", out_dir.display());
    println!("  run it with {}", launcher.display());
}

fn run(command: &mut Command, what: &str) {
    match command.status() {
        Ok(status) if status.success() => {}
        Ok(status) => {
            eprintln!("dartvel-cli-flt: {what} failed ({status})");
            exit(1);
        }
        Err(error) => {
            eprintln!("dartvel-cli-flt: could not run {what}: {error}");
            exit(1);
        }
    }
}

fn copy(from: &Path, to: &Path) {
    fs::copy(from, to)
        .unwrap_or_else(|e| panic!("copy {} -> {}: {e}", from.display(), to.display()));
}

fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).expect("create dir");
    for entry in fs::read_dir(from).expect("read dir") {
        let entry = entry.expect("dir entry");
        let target = to.join(entry.file_name());
        if entry.file_type().expect("file type").is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            copy(&entry.path(), &target);
        }
    }
}

/// icudtl.dat from the Flutter SDK's artifact cache.
///
/// `which flutter` gives the SDK root, and the file sits under
/// `bin/cache/artifacts/engine/<host>/`. Searched rather than hard-coded
/// because the host directory name differs by architecture.
fn find_icu_data() -> Option<PathBuf> {
    let flutter = Command::new("which").arg("flutter").output().ok()?;
    if !flutter.status.success() {
        return None;
    }
    let bin = PathBuf::from(String::from_utf8(flutter.stdout).ok()?.trim());
    // .../flutter/bin/flutter -> .../flutter
    let root = bin.parent()?.parent()?;
    find_file(&root.join("bin/cache/artifacts/engine"), "icudtl.dat")
}

fn find_file(root: &Path, name: &str) -> Option<PathBuf> {
    let entries = fs::read_dir(root).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if let Some(found) = find_file(&path, name) {
                return Some(found);
            }
        } else if path.file_name().map(|f| f == name).unwrap_or(false) {
            return Some(path);
        }
    }
    None
}
