# Sly

Sly is a cross-platform, multi-system emulator for players, homebrew developers and emulator developers alike. It combines a beautiful debugger with a unique take on the classic emulation experience!

[Official website](https://sly.layle.dev/) · [Try the debugger in your browser](https://sly.layle.dev/app)

<p>
  <img src="assets/screenshots/debugger.png" width="49%" alt="Sly Debugger">
  <img src="assets/screenshots/launcher.png" width="49%" alt="Sly Launcher">
</p>

Sly has three main desktop frontends, alongside its mobile apps:

* **Sly Launcher**: A 3D game library built around box art and cartridges.
* **Sly Classic**: A minimal Qt interface for browsing and playing games.
* **Sly Debugger**: A beautiful debugger packed with features.

Supported platforms:

* Windows, macOS and Linux
* Web browsers via WebGPU (debugger only so far)
* iOS, with Android planned

## Download

Get the [latest release](https://github.com/ioncodes/sly/releases/latest):

* **Windows, macOS and Linux**: each frontend comes as its own archive: `sly-launcher`, `sly-classic` and `sly-debugger`.
* **iOS**: `sly-ios-arm64-unsigned.ipa`. It's unsigned, so you need to sideload it (for example with AltStore or SideStore).

For the newest changes, try the [nightly build](https://github.com/ioncodes/sly/releases/tag/nightly). It's rebuilt from the `dev` branch once a day and may be unstable.

## Compatibility

Color means playable or work-in-progress. Click on a logo to see the respective compatibility database.

<table>
  <tr>
    <td><a href="https://emu.layle.dev/sly-snes"><img src="assets/systems/snes.svg" width="240" alt="Super Nintendo" title="Super Nintendo screenshot database"></a></td>
    <td><a href="https://emu.layle.dev/sly-gba"><img src="assets/systems/gba.svg" width="240" alt="Game Boy Advance" title="Game Boy Advance screenshot database"></a></td>
    <td><img src="assets/systems/gb.svg" width="240" alt="Game Boy and Game Boy Color"></td>
    <td><img src="assets/systems/gg.svg" width="240" alt="Game Gear"></td>
  </tr>
  <tr>
    <td><img src="assets/systems/sms.svg" width="240" alt="Master System"></td>
    <td><img src="assets/systems/psx.svg" width="240" alt="PlayStation"></td>
    <td><img src="assets/systems/gc.svg" width="240" alt="GameCube"></td>
    <td><img src="assets/systems/wii.svg" width="240" alt="Wii"></td>
  </tr>
  <tr>
    <td><img src="assets/systems/genesis.svg" width="240" alt="Sega Genesis"></td>
    <td><img src="assets/systems/dreamcast.svg" width="240" alt="Dreamcast"></td>
    <td><img src="assets/systems/nds.svg" width="240" alt="Nintendo DS"></td>
    <td><img src="assets/systems/3ds.svg" width="240" alt="Nintendo 3DS"></td>
  </tr>
</table>

## Features

**Sly Launcher**
* 3D boxes and cartridges
* Row, column and ribbon shelf layouts
* Flippable box art
* Automatic artwork downloads
* Regional packaging and models, with per-game release overrides
* Per-game cartridge finishes
* Gamepad navigation
* Remappable controls
* Save states
* Quality presets, text size, reduced motion and integer scaling
* Fullscreen

**Sly Classic**
* Sortable game list with banners
* Quick save slot plus ten save state slots
* 1x to 4x scaling and fullscreen
* Gamepad support
* Automatic artwork downloads

**Sly Debugger**
* Instruction and event stepping
* Linear and graph disassembly
* CPU registers and call stacks
* CPU and memory breakpoints
* Memory viewer
* MMIO register inspection
* DMA inspection
* Tile, tilemap and background viewers
* Sprite and palette viewers
* Various coprocessor workspaces (for example SA-1 and Super FX)
* Audio visualization
* Built-in Lua editor and scripting
* Save states
* Cartridge info with cover art
* Light and dark themes with various accent colors

## Usage

| System | File types     |
| ------ | -------------- |
| SNES   | `.sfc`, `.smc` |
| GBA    | `.gba`         |

How you load games depends on the frontend:

* **Sly Launcher**: add your ROM folders in settings, or pass a folder on the command line.
* **Sly Classic**: put your ROMs in `roms/{system}/` in the working directory, meaning the same directory that `sly-classic.exe` is in. For example, if your `.exe` file is `Sly/bin/sly-classic.exe`, then your game files should be `Sly/bin/roms/System-name/game.smc`
* **iOS**: copy games into Sly's `roms/{system}/` folders using the Files app.
* **Sly Debugger**: pass the ROM path on the command line, or drag it into the window.

From the command line:

```sh
sly-launcher path/to/roms/
sly-debugger path/to/game.sfc
```

## Artwork

Box art and covers come from [art.layle.dev](https://art.layle.dev). This works without an account, but anonymous access has lower rate limits. For higher limits, log in at [art.layle.dev/account](https://art.layle.dev/account), generate an API key and paste it into the settings of any frontend:

* **Sly Launcher**: Settings -> Artwork key
* **Sly Classic**: File -> Settings
* **Sly Debugger**: the settings popover, under Artwork
* **iOS**: Settings -> Artwork

The key is stored in `settings.toml` in Sly's data folder (for example `%APPDATA%\sly` on Windows), which all desktop frontends share.

## Default Controls

These are Sly Launcher's defaults. All of them can be remapped in settings (gamepads are supported too).

Keys are matched by position, so the labels below are for a US QWERTY keyboard. On other layouts, press the key in the same place (for example, **Z** is **Y** on a German keyboard).

**Library**

| Action                  | Key               |
| ----------------------- | ----------------- |
| Browse                  | Arrow keys / WASD |
| Play                    | Enter / Space     |
| Flip box                | F                 |
| Change view             | Tab               |
| Search                  | /                 |
| Previous / next letter  | Q / E             |
| Console list            | C                 |
| Previous / next console | Page Up / Down    |
| Settings                | F2                |
| Fullscreen              | F11               |

**In game**

| Control           | Key               |
| ----------------- | ----------------- |
| D-pad             | Arrow keys        |
| B / A             | Z / X             |
| Y / X (SNES)      | A / S             |
| L / R             | Q / W             |
| Start / Select    | Enter / Backspace |
| Save / load state | F5 / F8           |
| Pause             | P                 |
| Back to library   | Esc               |
| Fullscreen        | F11               |

## Development

### Building

See the [build workflow](.github/workflows/binaries.yml) for platform dependencies, including Linux audio and windowing libraries. Run everything from the repository root.

**Sly Launcher and Sly Debugger**

```sh
git submodule update --init --recursive
cargo run --locked -p modern-launcher -- roms/
cargo run --locked -p debugger -- path/to/game.sfc
```

This should work for most of the Rust crates.

**Sly Classic** also needs:

* Qt 6 (CI uses 6.11.1), including Multimedia, ShaderTools and private GUI headers
* CMake 4.3 or newer
* Ninja
* A C++23 compiler
* naga-cli 29.0.4 (`cargo install naga-cli --version 29.0.4 --locked`)
* SDL3 3.2 or newer for gamepads (or configure with `-DSLY_ENABLE_GAMEPAD=OFF`)

```sh
qt-cmake -S frontends/desktop-qt -B build/desktop-qt -G Ninja \
  -DCMAKE_BUILD_TYPE=Release -DSLY_CARGO_PROFILE=release
cmake --build build/desktop-qt
```

**iOS** needs a Mac with Xcode 26. Open `frontends/ios/Sly.xcodeproj`, choose your signing team and run the `Sly` scheme.

### Project layout

Emulated hardware lives in `crates/cores/`, the shared interfaces in `crates/models/` and the frontends in `frontends/`. If you want to add a system, the [GBA core](crates/cores/gba/src/lib.rs) is the easiest one to read. The [SNES core](crates/cores/snes/src/lib.rs) is a good example of multiple CPUs and cartridge coprocessors.

The crates you'll touch most:

* [core-model](crates/models/core/src/lib.rs): the `Core` trait every system implements for playback (frames, audio, input, save states).
* [debugger-model](crates/models/debugger/src/target.rs): `DebugCore`, which adds registers, disassembly, stepping, breakpoints and memory access on top of `Core`.
* [workspace-model](crates/models/workspace/src/lib.rs): describes the debugger's workspaces and panels.
* [core-loader](crates/core-loader/src/lib.rs): detects ROMs and creates the right core. New systems get hooked up here.
* [cirsium](crates/cirsium): the debugger's UI components.

### Adding a core

1. Create a crate in `crates/cores/<system>/` and add it to the [workspace](Cargo.toml). Copy the setup from the [GBA Cargo.toml](crates/cores/gba/Cargo.toml) if you want debugger support behind a `debug` feature.
2. Implement `core_model::Core` for your machine. Only `reset`, `step` and `run_frame` are required. Everything else has a default, so you can add video, audio, input and save states as you go.
3. Add the system to `core_model::System` and to the save-state tags in [state.rs](crates/models/core/src/state.rs). Don't change the existing tags.
4. Hook it up in [core-loader](crates/core-loader/src/lib.rs): `identity`, `inspect`, `load`, and `load_debug` once the debugger works.
5. Check the frontends and bindings for anything system-specific, like controls or assets.

While you're working on a new core, the [runner](frontends/runner/src/main.rs) is handy. It's a tiny SDL app that just loads a ROM and plays it, so there's much less in the way than with the other frontends. It only needs `cartridge_info` and `frame_interval` on top of the basics.

```sh
cargo run -p runner -- path/to/game.gba
```

### Adding debugger support

Wrap your machine in a debug type that implements both `Core` and `DebugCore`, like [GbaDebug](crates/cores/gba/src/debug.rs) or [SnesDebug](crates/cores/snes/src/debug.rs). Start with CPUs, registers, disassembly, memory regions and stepping. Call stacks, event stepping and scripting can come later.

A few things to keep in mind:

* Use the same IDs everywhere (for example `gba.arm7tdmi` and `gba.bus`). Breakpoints and views refer to them.
* `peek_memory` must not change the machine's state. `read_bus` / `write_bus` go through the bus with all side effects and `patch_memory` edits memory directly.
* The default `manifest` gives you the standard panels. Override it to add your own workspaces, like the [SNES core](crates/cores/snes/src/debug/manifest.rs) does for its coprocessors and call `ComponentManifest::validate()` to check it.

### Testing

```sh
cargo fmt --all --check
cargo clippy -p gba -p snes --features gba/debug,snes/debug
cargo test -p gba -p snes --features gba/debug,snes/debug
```

CPU tests run against the [SSTs](https://github.com/SingleStepTests) in `submodules/`:

```sh
cargo run --release -p testrunner -- snes
cargo run --release -p testrunner -- gba
cargo run --release -p testrunner -- spc700 --opcodes 00 --limit 100
```

New CPUs can be registered in [testrunner](crates/testrunner/src/cores.rs).

## Contributing

Contributions are welcome, whether it's a game compatibility fix, a new debugger feature or a whole new core. For bigger changes, open an issue first so we can talk about it before you sink time into it.

Sly also needs more than code! If you have design experience, help with app icons, UI and UX, box art and cartridge models or other visual assets would be very welcome. Open an issue to share your ideas or work.

For pull requests:

* Branch off `dev` and open your PR against it.
* Run `cargo fmt --all` and make sure the checks under [Testing](#testing) pass.
* Keep hardware changes in their core and match the style of the surrounding code.
* Explain what changed and how you tested it. If you ran the CPU tests or checked specific games, mention them.
* Add screenshots for anything visual.
* Don't commit ROMs, BIOS files or build output.

### LLM usage

You may use LLMs to help write your code, but I strongly recommend against it. Either way, you're expected to understand every change you submit and be able to explain it yourself. Don't act as a proxy for an LLM. Issues, PR descriptions and discussions must be written by you. LLM usage must be disclosed.

## License

[GPL-3.0](LICENSE) · [Third-party notices](licenses)

Sly is not affiliated with Nintendo or other console manufacturers. Game titles, artwork and trademarks belong to their respective owners.
