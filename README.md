> ## Dartvel fork
>
> This is [Dartvel](https://github.com/Danroyal001/dartvel)'s fork of
> [`jiahaog/flt`](https://github.com/jiahaog/flt), the Flutter terminal
> embedder — Rust, Flutter's Custom Embedder API, rendering through the Kitty
> graphics protocol with an ANSI fallback.
>
> **Why it exists.** Terminal rendering is a Dartvel build target:
> `dartvel build linux-cli` (alias `-tui`) produces a binary that renders in a
> terminal and contains no GUI backend at all. Upstream is a research project
> that **runs apps in development** and does not produce distributable
> binaries. Supplying that, and whatever else Dartvel needs, is this fork's
> job rather than a reason to wait — the same arrangement as the television and
> embedded embedder forks.
>
> **Status: re-pinned to Dartvel's Flutter and building.** The Dart side of
> terminal rendering — target resolution, build-time backend selection,
> `DV.Platform.surface`, launch negotiation — is implemented and tested in the
> main repository. What remains here is a distributable `build`, described
> below.
>
> **Verified against Flutter 3.44.5** (`f94f4fc76b`, engine
> `83675ed27633283e7fc296c8bca22e841224c096`) on linux-x64: the whole workspace
> compiles and links against that engine's prebuilt embedder library.
>
> Upstream pinned Flutter 3.38.5, which ships a Dart below Dartvel's floor of
> 3.12. That is the same wall webOS and Sony eLinux hit — but unlike them it
> came down without an engine build, because the prebuilt `linux-x64-embedder`
> artifact is published for Dartvel's engine. Re-pinning needed four source
> changes, all of them C structs that gained fields between the two versions:
>
> - `FlutterWindowMetricsEvent` gained view constraints. `has_constraints:
>   false` preserves the previous behaviour — an unconstrained view — so the
>   terminal keeps sizing itself from width/height.
> - `FlutterPointerEvent` gained stylus pressure. A terminal reports mouse
>   events and has no stylus, so the range is degenerate.
> - `FlutterProjectArgs` gained `enable_wide_gamut`. A colour-space feature for
>   real displays; off is both correct here and the prior behaviour.
> - The new constraint fields are physical pixels (`usize`), not logical
>   (`f64`), which the compiler caught and is worth writing down.
>
> Submodule URLs were changed from SSH to HTTPS. Dartvel installs this fork
> unattended and CI has no keys, so an SSH remote makes the clone fail for
> everyone who is not the upstream author.
>
> **What is still missing: a distributable build.** `flt-cli` is a development
> loop — it builds the app, compiles the embedder from source, and runs it —
> and upstream notes that the Flutter project is always built in debug mode
> (`TODO: Implement support for Flutter projects in AOT mode`). Dartvel needs a
> `dartvel-flt build <platform>` that emits an artifact someone can ship, and
> installs under that name; `dartvel doctor --target linux-cli` already looks
> for it at `~/.dartvel/toolchains/dartvel_flt/bin/dartvel-flt`. Until that
> exists, `dartvel build linux-cli` skips and says so rather than substituting
> a GUI build.
>
> Upstream documentation and licence follow, untouched.

# flt

`flt` is a **Fl**utter **T**erminal Embedder, implementing the Flutter Engine's [Custom Embedder API](https://docs.flutter.dev/embedded).

With a terminal emulator that [supports](https://sw.kovidgoyal.net/kitty/graphics-protocol/) Kitty graphics, 60fps rendering can be achieved.

https://github.com/user-attachments/assets/2e912395-204a-4a81-9aae-649e7f02b090

Otherwise, it falls back to using [ANSI Escape Codes](https://en.wikipedia.org/wiki/ANSI_escape_code).

https://github.com/user-attachments/assets/b6e58c93-4f30-43e4-b0e5-07e50947da9c

This works over SSH though it may be slow depending on the network.

## Supported Platforms / Terminals

Kitty rendering was mostly developed on macOS. Tested on iTerm2 and Ghostty.

ANSI rendering should work on more terminals.

## Checkout

This project uses submodules, so pass the `--recurse-submodules` flag.

```sh
git clone --recurse-submodules git@github.com:jiahaog/flt.git
```

## Usage

Install [Rust](https://www.rust-lang.org/tools/install) first, then at the root of the monorepo, the following command will build the [Sample Flutter App](./sample_app/), and run it with the terminal embedder.

```sh
cargo run
```

### Other Flutter Projects

```sh
cargo run -- <path to the root of your flutter project>
```

### Usage with `flutter run` (Custom Device)

The terminal embedder can be registered as a [Custom Device](https://github.com/flutter/flutter/blob/master/docs/tool/Using-custom-embedders-with-the-Flutter-CLI.md#the-custom-devices-config-file) to use it directly with the `flutter` tool (supporting hot reload, hot restart etc.).

1.  Enable Custom Devices:

    ```sh
    flutter config --enable-custom-devices
    ```

2.  Build the Embedder:

    ```sh
    cargo build --release
    ```

3.  Install Custom Device:

    Run the installation script to configure the custom device and launcher:
    ```sh
    ./install_custom_device.sh
    ```

4.  Run:
    ```sh
    flutter run -d terminal
    ```

### More CLI help for development

```sh
# See help for `flt-cli`.
cargo run -- --help

# See help for `flt`.
cargo run -- --args=--help
```

## Project Structure

- [`flt`](./flt) - The terminal embedder.
- [`flt-cli`](./flt-cli/) - A small CLI utility to make local development easier. By default, the `cargo run` command at the root of the repository will run this.
- [`flutter-sys`](./flutter-sys/) - Safe Rust bindings to the Flutter Embedder API.
- [`sample_app`](./sample_app/) - A sample Flutter Project used for local development.
- [`third_party/flutter`](./third_party/flutter/) - A submodule checkout of the [Flutter Framework](https://github.com/flutter/flutter).

## References

- [Forking Chrome to render in a terminal](https://fathy.fr/carbonyl)
- [brow.sh](https://www.brow.sh/)
