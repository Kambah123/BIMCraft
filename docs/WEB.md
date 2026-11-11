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

The 3D view is drawn on the graphics card on WebGPU and on WebGL2 alike. WebGL2 has no storage
buffers and its shaders can't read a depth texture, so there the card draws with
`crates/ui-egui/src/gpu_webgl.wgsl`: the elements' colours, the textures' lookups and texels and the
copies' plane numbers are 2D textures of four 32-bit words a texel, the levels' lines are an instance
vertex buffer, and the opaque depth is written into the id target as well as tested in the depth
target. WebGPU and the desktop read the same tables from storage buffers (`gpu_storage.wgsl`); both
share `gpu.wgsl`. The tests in `gpu.rs` that compare the card's picture with the processor's pass on
OpenGL (Mesa llvmpipe) with WebGL2's limits. The browser console says which way the 3D view is drawn
(`bimcraft: 3D view on the graphics card: … (Gl, WebGL2 (tables in textures))`). Add `?3d=cpu` to the
address to keep it on the processor, and `?perf3d` to log how long its pictures take.

Background view jobs (the 3D scene, sections, plans, the upload to the card) run on the window's
thread in the browser, as there are no threads to start there.

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

## Accounts and cloud save (CampusBIM)

In the browser the app bar has a **Sign in** chip, left of the Palettes button, and File has
**Save to Cloud**, **My Projects…** and **Sign In… / Sign Out**. This is browser-only
(`crates/ui-egui/src/cloud.rs`, compiled for `wasm32` only); the desktop app has none of it and
does not reach the network for it.

- **Accounts.** Email and password, on Supabase Auth (`/auth/v1/signup`, `/auth/v1/token`). The
  dialog has a Sign in and a Create account tab. If the project asks for email confirmation,
  sign-up says "Check your email to confirm your account, then sign in" instead of signing in.
  The session (access and refresh tokens) is kept in `localStorage` under `campusbim.auth` and
  refreshed about five minutes before it runs out, and again just before any request whose
  token has under a minute left. Sign Out forgets it and revokes it on the server.
- **Save to Cloud** encodes the project as on File > Save (`Session::encode("bimcraft")`),
  uploads it to the private Storage bucket `projects` at `<user id>/<project id>.bimcraft`, and
  upserts its row in `public.projects` (name, size, path; `updated_at` is set by a trigger).
  Saving again a project that was saved or opened from the cloud overwrites the same entry. Not
  signed in, it asks to sign in first and then saves. It marks the project saved.
- **My Projects** lists the user's projects (newest first) with Open and Delete (Delete asks
  first). Open downloads the file and opens it through `Session::provide_bytes`, the same path
  as File > Open; Delete removes the file and the row.
- **Recovery** (IndexedDB, every 30 seconds) is unchanged and independent of the cloud.

The database (the `projects` table, its Row Level Security and the bucket's policies) is
`supabase/migrations/20261111000000_projects_and_storage.sql`. Each user reads and writes only their
own rows and only files under their own `<user id>/` folder; the anonymous role has no access to
the table. The client talks to Supabase's HTTP APIs with the page's own `fetch` (web-sys), so no
HTTP client crate is added. The project URL and the publishable key are built in; they are
public by design. `CAMPUSBIM_SUPABASE_URL` and `CAMPUSBIM_SUPABASE_KEY` at build time point the
build at another project. Never put a service-role key in the client.

`window.__bimcraftCloud` is a JSON string of the account and cloud state (signed in, email, the
open cloud project, the last listed projects and the last message; never the tokens), for tests.

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
- **Cloud.** One file per project, last save wins: no sharing, no version history, and no
  live collaboration yet. Password reset and email change are not in the dialog.
- **The agent port.** The TCP control port (`--control`) assumes a localhost listener.
- **Background work.** View jobs run on the window's thread (a large model pauses the page while its
  3D scene is built); Revit import on a thread and rayon's render pool do not get threads here.
- **Size.** A release `trunk build` of this tree is 38,601,800 bytes (38.6 MB) of Wasm without
  wasm-opt, and 28,035,288 bytes (28.0 MB) after `wasm-opt -z`. The file I/O, download and
  recovery code accounts for most of the growth from the previous WebGL-fallback tree.

The updater, the DWG child, the SpaceMouse and the MCP TCP port should stay desktop-only.
