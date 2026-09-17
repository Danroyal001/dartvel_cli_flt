//! `dartvel-cli-flt` — producing a terminal application someone can ship.
//!
//! This is the half upstream does not have, and the reason Dartvel forks this
//! repository. `flt-cli` is a development loop: it builds the Flutter project,
//! compiles the embedder from source, and runs it, all against a checkout. It
//! never emits an artifact that can leave the machine.
//!
//! Dartvel needs one. `dartvel build linux-cli`, `macos-cli` and `windows-cli`
//! promise a binary that renders in a terminal and contains no GUI backend,
//! and their preflight looks for this executable by name under
//! `~/.dartvel/toolchains/dartvel_cli_flt/bin/`.
//!
//! It builds for the host it runs on. The embedder links a host engine, and
//! each host lays its bundle out differently -- see [host_layout].
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

/// How the engine arrives from `flutter-sys`'s build.
#[derive(Debug, PartialEq, Eq)]
enum Engine {
    /// A single library file of this name.
    SharedLibrary(&'static str),
    /// `FlutterEmbedder.framework`, a directory with symlinks inside.
    Framework,
}

/// What a bundle looks like on one host.
#[derive(Debug, PartialEq, Eq)]
struct HostLayout {
    /// `flutter build bundle --target-platform`.
    flutter_target_platform: &'static str,
    /// The embedder binary cargo builds, and its name in the bundle.
    executable: &'static str,
    engine: Engine,
    /// Where the engine goes, relative to the bundle root. Each is where the
    /// binary's loader looks: `$ORIGIN/lib` on Linux, `@executable_path/lib`
    /// on macOS, and the executable's own directory on Windows.
    engine_destination: &'static str,
    launcher: &'static str,
}

/// The bundle layout for `os`/`arch`, spelled as `std::env::consts` spells
/// them.
fn host_layout(os: &str, arch: &str) -> Result<HostLayout, String> {
    let arch_name = match arch {
        "x86_64" => "x64",
        "aarch64" => "arm64",
        other => return Err(format!("no terminal bundle for the {other} architecture on {os}")),
    };
    match os {
        "linux" => Ok(HostLayout {
            flutter_target_platform: if arch_name == "x64" { "linux-x64" } else { "linux-arm64" },
            executable: "flt",
            engine: Engine::SharedLibrary("libflutter_engine.so"),
            engine_destination: "lib/libflutter_engine.so",
            launcher: "run.sh",
        }),
        // The Mac engine is one universal framework, and `flutter build bundle`
        // has a single `darwin` platform for both architectures.
        "macos" => Ok(HostLayout {
            flutter_target_platform: "darwin",
            executable: "flt",
            engine: Engine::Framework,
            engine_destination: "lib/FlutterEmbedder.framework",
            launcher: "run.sh",
        }),
        "windows" => Ok(HostLayout {
            flutter_target_platform: if arch_name == "x64" { "windows-x64" } else { "windows-arm64" },
            executable: "flt.exe",
            engine: Engine::SharedLibrary("flutter_engine.dll"),
            engine_destination: "flutter_engine.dll",
            launcher: "run.cmd",
        }),
        other => Err(format!(
            "no terminal bundle for {other}; dartvel-cli-flt builds on linux, macos and windows"
        )),
    }
}

/// The launcher written beside the binary.
///
/// It exists because a bundle should run from anywhere, not only from a shell
/// that already knows where its pieces are. It also sends the application's
/// own output to a log file: the terminal is the display here, so it cannot
/// also be the console, and a screenshot of a running bundle once showed the
/// frame shredded by interleaved `print` lines.
fn launcher_script(layout: &HostLayout) -> String {
    let exe = layout.executable;
    if layout.launcher.ends_with(".cmd") {
        [
            "@echo off",
            "rem Runs the terminal application from this directory.",
            "setlocal",
            "set \"here=%~dp0\"",
            "if \"%FLT_LOG_FILE%\"==\"\" set \"FLT_LOG_FILE=%here%app.log\"",
            &format!(
                "\"%here%{exe}\" --assets-dir \"%here%data\\flutter_assets\" \
                 --icu-data-path \"%here%data\\icudtl.dat\" --log-file \"%FLT_LOG_FILE%\" %*"
            ),
            "exit /b %ERRORLEVEL%",
            "",
        ]
        .join("\r\n")
    } else {
        // Linux still sets LD_LIBRARY_PATH beside the binary's $ORIGIN/lib
        // runpath, so a bundle keeps working with an flt built before the
        // runpath was. macOS does not: SIP strips DYLD_* from anything started
        // through /bin/sh, and flt carries @executable_path/lib instead.
        let library_path = match layout.engine {
            Engine::Framework => "",
            Engine::SharedLibrary(_) => "LD_LIBRARY_PATH=\"$here/lib:$LD_LIBRARY_PATH\" \\\n",
        };
        format!(
            "#!/bin/sh\n\
             # Runs the terminal application from this directory.\n\
             here=$(cd \"$(dirname \"$0\")\" && pwd)\n\
             {library_path}\
             exec \"$here/{exe}\" \\\n\
             \x20 --assets-dir \"$here/data/flutter_assets\" \\\n\
             \x20 --icu-data-path \"$here/data/icudtl.dat\" \\\n\
             \x20 --log-file \"${{FLT_LOG_FILE:-$here/app.log}}\" \"$@\"\n"
        )
    }
}

fn host_platform_name() -> &'static str {
    match env::consts::OS {
        "macos" => "macos",
        "windows" => "windows",
        _ => "linux",
    }
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

    let host = host_platform_name();
    if let Some(platform) = platform.as_deref() {
        if platform != host {
            eprintln!(
                "dartvel-cli-flt: cannot build for {platform} on {host}. The embedder \
                 links a host engine; cross-building is not supported."
            );
            exit(2);
        }
    }

    let layout = host_layout(env::consts::OS, env::consts::ARCH).unwrap_or_else(|reason| {
        eprintln!("dartvel-cli-flt: {reason}");
        exit(2);
    });

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
    run(
        flutter()
            .current_dir(&project_dir)
            .args(["build", "bundle", "--target-platform", layout.flutter_target_platform]),
        "flutter build bundle",
    );

    // Then the embedder, optimised — it is the binary being shipped.
    run(
        Command::new("cargo")
            .current_dir(&workspace)
            .args(["build", "--release", "-p", "flt"]),
        "cargo build -p flt",
    );

    let build_dir = workspace.join("target/release/build");
    let engine_name = match layout.engine {
        Engine::SharedLibrary(name) => name,
        Engine::Framework => "FlutterEmbedder.framework",
    };
    let engine = find_engine(&build_dir, engine_name).unwrap_or_else(|| {
        eprintln!(
            "dartvel-cli-flt: {engine_name} was not found under {}.\n\
             It is downloaded by flutter-sys during the build; a missing \
             one means that build did not run.",
            build_dir.display()
        );
        exit(1);
    });

    let out_dir = project_dir.join(&out);
    let data_dir = out_dir.join("data");
    // A previous bundle is replaced, not merged into: a framework copied over
    // an older one keeps whatever the new one no longer has.
    let engine_destination = out_dir.join(layout.engine_destination);
    if engine_destination.is_dir() {
        fs::remove_dir_all(&engine_destination).expect("remove previous engine");
    }
    fs::create_dir_all(&data_dir).expect("create data dir");
    fs::create_dir_all(engine_destination.parent().unwrap()).expect("create engine dir");

    copy(
        &workspace.join("target/release").join(layout.executable),
        &out_dir.join(layout.executable),
    );
    match layout.engine {
        Engine::SharedLibrary(_) => copy(&engine, &engine_destination),
        Engine::Framework => copy_dir(&engine, &engine_destination),
    }

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
    // The Mac framework carries its own, which matches the engine exactly;
    // elsewhere it lives in the Flutter SDK's artifact cache.
    let icu = match layout.engine {
        Engine::Framework => Some(engine.join("Resources/icudtl.dat")).filter(|p| p.is_file()),
        Engine::SharedLibrary(_) => find_icu_data(),
    };
    match icu {
        Some(icu) => copy(&icu, &data_dir.join("icudtl.dat")),
        None => {
            eprintln!(
                "dartvel-cli-flt: icudtl.dat was not found.\n\
                 The engine cannot start without it, so the bundle would be \
                 assembled and dead."
            );
            exit(1);
        }
    }

    let launcher = out_dir.join(layout.launcher);
    fs::write(&launcher, launcher_script(&layout)).expect("write launcher");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&launcher, fs::Permissions::from_mode(0o755))
            .expect("chmod launcher");
    }

    println!("dartvel-cli-flt: wrote {}", out_dir.display());
    println!("  run it with {}", launcher.display());
}

/// `flutter`, runnable on this host: `flutter.bat` on Windows, which
/// `Command::new("flutter")` does not find because it looks only for `.exe`.
fn flutter() -> Command {
    if cfg!(windows) {
        let mut command = Command::new("cmd");
        command.args(["/C", "flutter"]);
        command
    } else {
        Command::new("flutter")
    }
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

/// Copies a directory, keeping symlinks as symlinks.
///
/// A framework is built from them -- `FlutterEmbedder -> Versions/Current/...`
/// -- and following them would either fail on the directory links or ship the
/// 90 MB engine twice.
fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).expect("create dir");
    for entry in fs::read_dir(from).expect("read dir") {
        let entry = entry.expect("dir entry");
        let target = to.join(entry.file_name());
        let file_type = entry.file_type().expect("file type");
        if file_type.is_symlink() {
            copy_symlink(&entry.path(), &target);
        } else if file_type.is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            copy(&entry.path(), &target);
        }
    }
}

#[cfg(unix)]
fn copy_symlink(from: &Path, to: &Path) {
    let link = fs::read_link(from).expect("read link");
    let _ = fs::remove_file(to);
    std::os::unix::fs::symlink(link, to).expect("symlink");
}

#[cfg(not(unix))]
fn copy_symlink(from: &Path, to: &Path) {
    if from.is_dir() {
        copy_dir(from, to);
    } else {
        copy(from, to);
    }
}

/// The engine from the newest `flutter-sys` build output.
///
/// Newest, because the build directory keeps every output it has ever made: an
/// engine revision change leaves the old one beside the new, and the first
/// directory listed is not the one just linked.
fn find_engine(build_dir: &Path, name: &str) -> Option<PathBuf> {
    fs::read_dir(build_dir)
        .ok()?
        .flatten()
        .filter(|entry| entry.file_name().to_string_lossy().starts_with("flutter-sys-"))
        .map(|entry| entry.path().join("out").join(name))
        .filter(|path| path.exists())
        .max_by_key(|path| fs::metadata(path).and_then(|m| m.modified()).ok())
}

/// icudtl.dat from the Flutter SDK's artifact cache.
///
/// The SDK root comes from `flutter --version --machine`, which works the same
/// on every host -- `which` does not exist on Windows. The file sits under
/// `bin/cache/artifacts/engine/<host>/`, searched rather than hard-coded
/// because the host directory name differs by architecture.
fn find_icu_data() -> Option<PathBuf> {
    let output = flutter().args(["--version", "--machine"]).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let root = PathBuf::from(json_string_field(&text, "flutterRoot")?);
    find_file(&root.join("bin/cache/artifacts/engine"), "icudtl.dat")
}

/// A string field out of flat JSON, without a JSON dependency.
fn json_string_field(text: &str, key: &str) -> Option<String> {
    let quoted = format!("\"{key}\"");
    let rest = &text[text.find(&quoted)? + quoted.len()..];
    let rest = rest.trim_start().strip_prefix(':')?.trim_start().strip_prefix('"')?;
    let mut value = String::new();
    let mut chars = rest.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => return Some(value),
            '\\' => value.push(chars.next()?),
            c => value.push(c),
        }
    }
    None
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_ships_flt_exe_with_the_engine_dll_beside_it() {
        let layout = host_layout("windows", "x86_64").unwrap();
        assert_eq!(layout.flutter_target_platform, "windows-x64");
        assert_eq!(layout.executable, "flt.exe");
        // Windows searches the executable's own directory for a DLL first, so
        // beside it is the one place that needs no search path.
        assert_eq!(layout.engine, Engine::SharedLibrary("flutter_engine.dll"));
        assert_eq!(layout.engine_destination, "flutter_engine.dll");
        assert_eq!(layout.launcher, "run.cmd");
    }

    #[test]
    fn windows_arm64_bundles_for_windows_arm64() {
        let layout = host_layout("windows", "aarch64").unwrap();
        assert_eq!(layout.flutter_target_platform, "windows-arm64");
    }

    #[test]
    fn the_windows_launcher_is_a_batch_file_that_runs_the_bundle() {
        let script = launcher_script(&host_layout("windows", "x86_64").unwrap());
        assert!(script.starts_with("@echo off\r\n"), "{script}");
        assert!(script.contains("\"%here%flt.exe\""), "{script}");
        assert!(script.contains("data\\flutter_assets"), "{script}");
        assert!(script.contains("data\\icudtl.dat"), "{script}");
        assert!(script.contains("%*"), "arguments are passed through: {script}");
        assert!(!script.contains("LD_LIBRARY_PATH"), "{script}");
        // cmd reads a batch file with LF endings, until a label or a goto
        // lands mid-line; CRLF is what it is written for.
        assert!(!script.replace("\r\n", "").contains('\n'), "{script}");
    }

    #[test]
    fn macos_ships_the_framework_under_lib_for_both_architectures() {
        for arch in ["x86_64", "aarch64"] {
            let layout = host_layout("macos", arch).unwrap();
            // `flutter build bundle` names the Mac `darwin`, not darwin-x64.
            assert_eq!(layout.flutter_target_platform, "darwin", "{arch}");
            assert_eq!(layout.executable, "flt", "{arch}");
            assert_eq!(layout.engine, Engine::Framework, "{arch}");
            // Where flt's @executable_path/lib rpath looks.
            assert_eq!(
                layout.engine_destination, "lib/FlutterEmbedder.framework",
                "{arch}"
            );
            assert_eq!(layout.launcher, "run.sh", "{arch}");
        }
    }

    #[test]
    fn the_macos_launcher_does_not_rely_on_a_library_path() {
        // DYLD_LIBRARY_PATH is stripped by SIP from anything launched through
        // /bin/sh, so a launcher that depended on it would work in a test and
        // fail on a user's machine.
        let script = launcher_script(&host_layout("macos", "aarch64").unwrap());
        assert!(script.starts_with("#!/bin/sh\n"), "{script}");
        assert!(!script.contains("LIBRARY_PATH"), "{script}");
        assert!(script.contains("\"$here/flt\""), "{script}");
    }

    #[test]
    fn linux_keeps_its_layout_and_follows_the_architecture() {
        let layout = host_layout("linux", "x86_64").unwrap();
        assert_eq!(layout.flutter_target_platform, "linux-x64");
        assert_eq!(layout.engine, Engine::SharedLibrary("libflutter_engine.so"));
        assert_eq!(layout.engine_destination, "lib/libflutter_engine.so");
        assert!(launcher_script(&layout).contains("LD_LIBRARY_PATH"));
        assert_eq!(
            host_layout("linux", "aarch64").unwrap().flutter_target_platform,
            "linux-arm64"
        );
    }

    #[test]
    fn an_unsupported_host_is_refused_by_name() {
        assert!(host_layout("freebsd", "x86_64").unwrap_err().contains("freebsd"));
    }

    #[test]
    fn the_flutter_root_is_read_from_the_machine_version() {
        let text = r#"{
  "frameworkVersion": "3.44.5",
  "flutterRoot": "C:\\hostedtoolcache\\flutter",
  "engineRevision": "83675ed27633283e7fc296c8bca22e841224c096"
}"#;
        assert_eq!(
            json_string_field(text, "flutterRoot").as_deref(),
            Some("C:\\hostedtoolcache\\flutter")
        );
        assert_eq!(json_string_field(text, "missing"), None);
    }
}
