# A browser build of BIMCraft

This is an experimental WebAssembly build of the desktop window, not a finished web app. It is
built with [Trunk](https://trunkrs.dev/) and eframe's `WebRunner`. The ordinary product remains
the native desktop app and the CLI.

## Build and run

From the repository root (Rust stable, the `wasm32-unknown-unknown` target, and `trunk`):

```sh
rustup target add wasm32-unknown-unknown
cargo install trunk
trunk serve --release
```

`trunk build --release` writes `dist/`. Serve that folder with any static file server. The page
needs a browser with [WebGPU](https://developer.mozilla.org/en-US/docs/Web/API/WebGPU_API)
(eframe's default renderer is wgpu). There is no WebGL fallback in this build.

The wasm check of the core and the CLI, which this front end depends on, is `ci/wasm-check.sh`.

## What works today

The window is the same egui shell as the desktop (`BimApp`). It starts on the sample house (the
one `bimcraft --sample` builds); there is no command line in the browser. Orbiting, panning and
zooming the 3D view, drawing in plan, and running commands from the ribbon all go through the
same session as on the desktop, in memory, once WebGPU is up.

The graphics card is used for the 3D view when WebGPU is available, as on the desktop.

On this machine, a headless Chrome load of `dist/` (Playwright, system Google Chrome) showed:

- The page and the Wasm module load (`index.html` 200, the `.wasm` and `.js` 200). The canvas is
  resized to the viewport, and the console logs `bimcraft: eframe started`.
- With WebGPU enabled via SwiftShader (`--enable-unsafe-webgpu` and ANGLE/SwiftShader), there was
  no panic and no page error. Headless screenshots and canvas readback stayed blank (a solid
  clear, or transparent), so this environment could not confirm that the UI actually painted.
  That is a known limitation of headless WebGPU, not proof that a real GPU browser would be blank.
- Without a WebGPU adapter, eframe still starts, then panics in wgpu while creating the `canvas`
  render pipeline: the shader's group-0 binding-0 is 24 bytes, and the downlevel device requires
  16-byte alignment. The only other console noise in both cases was a missing `/favicon.ico`
  (404). Do not expect the window to run in a browser that has no WebGPU.

`std::time::SystemTime::now` / `Instant::now` and `std::process::id` panic on
`wasm32-unknown-unknown` ("time not implemented", "no pids on this platform"). Fresh ids and the
clocks the core and the UI use go through `bimcraft_core::time` (web-time on wasm) and
`process_id()` instead.

## What does not work yet

These are current limitations, not accidental omissions:

- **Files.** Open, Save, Import and Export still talk to a filesystem path. The web file picker
  compiles, but it only keeps the file's name, not its bytes, and the core still uses `std::fs`.
  A project cannot actually be opened from or saved to disk in the browser.
- **Updates and the assistant.** The desktop updater and the AI chat (`ureq`, rustls, ring) are
  behind the `native-services` feature, which the wasm build turns off. rustls does not compile
  for `wasm32-unknown-unknown` as used here.
- **DWG.** The DWG reader runs in a child process of the desktop app; that path is not in the
  wasm build (`--no-default-features` also leaves out the Revit reader, though that crate itself
  type-checks for wasm).
- **Plugins, autosave, and the agent port.** The plugin folder, autosave and the TCP control port
  (`--control`) assume a local machine: threads, a writable config directory, and a localhost
  listener.
- **Background work.** Section and 3D workers, Revit import on a thread, and rayon's render pool
  type-check, but `std::thread` does not do useful work in this wasm target.
- **Size.** A release `trunk build` of this tree is 36,099,580 bytes (36.1 MB) of Wasm without
  wasm-opt, and 26,218,959 bytes (26.2 MB) after `wasm-opt -z` (about 50 seconds here). That is
  still large for a web page.

A later pass needs a real browser file API (bytes into `open` / `save` / IFC import), a smaller
download, and a decision on WebGL for browsers without WebGPU. The updater, the DWG child, the
SpaceMouse and the MCP TCP port should stay desktop-only.
