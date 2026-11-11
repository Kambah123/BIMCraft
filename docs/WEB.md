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
prefers [WebGPU](https://developer.mozilla.org/en-US/docs/Web/API/WebGPU_API) (eframe's wgpu
renderer). If the browser has no WebGPU adapter, wgpu falls back to WebGL2 (`webgl` is on for
the wasm build only; the desktop app stays on wgpu and does not change).

The wasm check of the core and the CLI, which this front end depends on, is `ci/wasm-check.sh`.

## What works today

The window is the same egui shell as the desktop (`BimApp`). It starts on the sample house (the
one `bimcraft --sample` builds); there is no command line in the browser. Orbiting, panning and
zooming the 3D view, drawing in plan, and running commands from the ribbon all go through the
same session as on the desktop, in memory.

The 3D view is drawn on the graphics card when WebGPU is available, as on the desktop. On
WebGL2, that path is not used: the shaders need storage buffers and a depth texture, which the
OpenGL backend cannot give them (`Gpu::new` already refuses `Backend::Gl` and the picture is
drawn on the processor instead). The 2D canvas's retained triangles can still be drawn through
wgpu on WebGL once their uniform is a multiple of 16 bytes.

`std::time::SystemTime::now` / `Instant::now` and `std::process::id` panic on
`wasm32-unknown-unknown` ("time not implemented", "no pids on this platform"). Fresh ids and the
clocks the core and the UI use go through `bimcraft_core::time` (web-time on wasm) and
`process_id()` instead.

## Files in the browser

Open, Save, Save As, Import and Export use the browser File API, not `std::fs`. The file picker
(`rfd`) yields a name and bytes; the core commands (`open`, `import_cad`) accept those bytes when
they are parked with `Session::provide_bytes`. Saving and exporting encode the project in memory
(`Session::encode`) and start a download (a Blob and a temporary `<a download>`, or `rfd`'s
writable handle when Save As used the picker).

What works:

- **Open** a `.bimcraft` project or an **IFC** file (File > Open, or drop the file on the window).
- **Save** and **Save As** download the project as `.bimcraft`. Save after opening a project
  re-downloads under the same name; a new or imported-IFC project asks for a name first.
- **Export** IFC, the current view as PNG, a plan as SVG, the sheets as PDF, and the 3D scene as
  glTF (`.glb`), same as the desktop File > Export menu.
- **Import CAD** for a **DXF** (Insert > Import CAD, or drop a `.dxf` on the window). The drawing
  is stored in the project.
- **Drag and drop** a `.bimcraft`, `.ifc` or `.dxf` onto the canvas.
- **Recovery.** About every 30 seconds, if the project has unsaved changes, a copy is written to
  IndexedDB (and to `localStorage` when it is small enough). On the next visit the window offers
  to restore it, so a drop in the network does not lose the work.

A test (or any page script) can open a file by setting `#bimcraft-file-input`, or by calling
`window.__bimcraftQueueFile(name, Uint8Array)`. `window.__bimcraftExport("ifc")` (or `bimcraft`,
`png`, `svg`, `pdf`, `glb`) encodes and downloads that format. `window.__bimcraftReady` is `true`
once eframe has started.

## What does not work yet

These are current limitations, not accidental omissions. Each one is refused with a status-bar
message; nothing fails silently or panics:

- **DWG.** The DWG reader runs as a child process of the desktop app; it is not in the wasm build.
  Save the drawing as DXF and import that.
- **Link CAD and Link Model.** A link is a filesystem path that is read again when the project
  opens or on Reload. Import the file into the project instead.
- **Point clouds, Revit (`.rvt` / `.rfa`), plugins from a folder, and recent files.** These
  assume a local path. Open a `.bimcraft` or IFC, or import a DXF, instead.
- **Updates and the assistant.** The desktop updater and the AI chat (`ureq`, rustls, ring) are
  behind the `native-services` feature, which the wasm build turns off. rustls does not compile
  for `wasm32-unknown-unknown` as used here.
- **The agent port.** The TCP control port (`--control`) assumes a localhost listener.
- **Background work.** Section and 3D workers, Revit import on a thread, and rayon's render pool
  type-check, but `std::thread` does not do useful work in this wasm target.
- **Size.** A release `trunk build` of this tree is 38,601,800 bytes (38.6 MB) of Wasm without
  wasm-opt, and 28,035,288 bytes (28.0 MB) after `wasm-opt -z`. The file I/O, download and
  recovery code accounts for most of the growth from the previous WebGL-fallback tree.

The updater, the DWG child, the SpaceMouse and the MCP TCP port should stay desktop-only.
