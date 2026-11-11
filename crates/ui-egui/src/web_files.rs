//! Browser file I/O: File API / drops into the session, downloads, and an IndexedDB recovery copy.
//! Native builds get empty stubs so call sites stay shared.

#[cfg(not(target_arch = "wasm32"))]
use std::path::Path;
#[cfg(target_arch = "wasm32")]
use std::path::PathBuf;

use egui::Context;

use crate::BimApp;
#[cfg(target_arch = "wasm32")]
use crate::Status;
#[cfg(target_arch = "wasm32")]
use crate::lang::t;

/// Park a file the page chose (hidden `<input>`, a hook a test calls) to be opened this frame.
pub fn queue_file(name: String, bytes: Vec<u8>) {
    QUEUE.with(|q| q.borrow_mut().push((name, bytes)));
    wake();
}

/// Ask the window to encode and download `ext` (bimcraft, ifc, glb, pdf, svg, png) this frame.
pub fn queue_export(ext: String) {
    EXPORTS.with(|q| q.borrow_mut().push(ext));
    wake();
}

thread_local! {
    static QUEUE: std::cell::RefCell<Vec<(String, Vec<u8>)>> = const { std::cell::RefCell::new(Vec::new()) };
    static EXPORTS: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
    static REPAINT: std::cell::RefCell<Option<egui::Context>> = const { std::cell::RefCell::new(None) };
    #[cfg(target_arch = "wasm32")]
    static SAVE_HANDLE: std::cell::RefCell<Option<rfd::FileHandle>> = const { std::cell::RefCell::new(None) };
    #[cfg(target_arch = "wasm32")]
    static RESTORE: std::cell::RefCell<Option<Snapshot>> = const { std::cell::RefCell::new(None) };
    #[cfg(target_arch = "wasm32")]
    static RESTORE_CHECKED: std::cell::RefCell<bool> = const { std::cell::RefCell::new(false) };
    #[cfg(target_arch = "wasm32")]
    static LAST_AUTOSAVE: std::cell::RefCell<Option<bimcraft_core::time::Instant>> = const { std::cell::RefCell::new(None) };
}

#[cfg(target_arch = "wasm32")]
#[derive(Clone)]
struct Snapshot {
    name: String,
    text: String,
    saved_at: f64,
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn stash_save_handle(handle: rfd::FileHandle) {
    SAVE_HANDLE.with(|h| *h.borrow_mut() = Some(handle));
}

#[cfg(target_arch = "wasm32")]
fn take_save_handle() -> Option<rfd::FileHandle> {
    SAVE_HANDLE.with(|h| h.borrow_mut().take())
}

/// Drain the file and export queues, run a download, offer a recovery copy, and periodically
/// snapshot the project. Call once a frame from the window.
pub fn poll(app: &mut BimApp, ctx: &Context) {
    REPAINT.with(|c| *c.borrow_mut() = Some(ctx.clone()));
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = (app, ctx);
    }
    #[cfg(target_arch = "wasm32")]
    {
        poll_wasm(app, ctx);
    }
}

#[cfg(target_arch = "wasm32")]
fn take_queued() -> Vec<(String, Vec<u8>)> {
    QUEUE.with(|q| std::mem::take(&mut *q.borrow_mut()))
}

#[cfg(target_arch = "wasm32")]
fn take_exports() -> Vec<String> {
    EXPORTS.with(|q| std::mem::take(&mut *q.borrow_mut()))
}

/// Ask the window to draw a frame so queued files and exports are handled.
pub fn wake() {
    REPAINT.with(|c| {
        if let Some(ctx) = c.borrow().as_ref() {
            ctx.request_repaint();
        }
    });
}

impl BimApp {
    /// Open or import a file we already have the bytes for, by its name's extension.
    #[cfg(target_arch = "wasm32")]
    pub(crate) fn open_named_bytes(&mut self, name: &str, bytes: Vec<u8>) {
        let path = PathBuf::from(name);
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
        self.session.provide_bytes(&path, bytes);
        match ext.as_str() {
            "dxf" => self.import_cad_path(path, false),
            "dwg" => {
                #[cfg(target_arch = "wasm32")]
                {
                    self.status = Status {
                        text: t(
                            "DWG isn't available in the browser: the reader runs as a desktop process. Save the drawing as DXF and import that.",
                        )
                        .into(),
                        error: true,
                    };
                }
                #[cfg(not(target_arch = "wasm32"))]
                {
                    self.import_cad_path(path, false);
                }
            }
            _ => {
                self.opens.push(path);
            }
        }
    }

    /// Why a desktop-only file command is refused in the browser, shown in the status bar. Returns true on wasm.
    pub(crate) fn web_block(&mut self, why: &str) -> bool {
        #[cfg(target_arch = "wasm32")]
        {
            self.status = Status { text: t(why).into(), error: true };
            true
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let _ = why;
            false
        }
    }
}

#[cfg(target_arch = "wasm32")]
fn poll_wasm(app: &mut BimApp, ctx: &Context) {
    for (name, bytes) in take_queued() {
        app.open_named_bytes(&name, bytes);
    }
    for ext in take_exports() {
        let stem = app.session.model.name.replace(['/', '\\'], "-");
        let path = PathBuf::from(format!("{stem}.{ext}"));
        let _ = app.save_to(&path);
    }
    restore_ui(app, ctx);
    autosave_tick(app);
}

#[cfg(target_arch = "wasm32")]
fn restore_ui(app: &mut BimApp, ctx: &Context) {
    use crate::popup::Modal;
    use bimcraft_core::time::Instant;
    use egui::{Align, Id as UiId, Layout, RichText};

    RESTORE_CHECKED.with(|c| {
        if !*c.borrow() {
            *c.borrow_mut() = true;
            // Init the autosave clock the first time we poll.
            LAST_AUTOSAVE.with(|t| *t.borrow_mut() = Some(Instant::now()));
            load_snapshot(ctx.clone());
        }
    });

    let Some(snap) = RESTORE.with(|r| r.borrow().clone()) else { return };
    let when = js_ms_to_local(&snap.saved_at);
    let (mut restore, mut discard) = (false, false);
    let title = t("Restore recovery copy?");
    let r = Modal::new(UiId::new("web-restore"), title).width(440.0).show(ctx, |ui| {
        crate::popup::body(ui, |ui| {
            ui.add_space(6.0);
            ui.label(RichText::new(t("A copy of this project was kept in this browser, so a drop in the network doesn't lose the work.")));
            ui.add_space(8.0);
            ui.label(format!("{} · {}", snap.name, when));
            ui.add_space(6.0);
            ui.label(t("Restore it in place of the sample house, or keep what is on screen and discard the copy."));
        });
        crate::popup::footer(ui, |ui| {
            ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                restore = ui.button(t("Restore")).clicked();
                discard = ui.button(t("Keep what's on screen")).clicked();
            });
        });
    });
    if restore {
        let path = PathBuf::from(&snap.name);
        app.session.provide_bytes(&path, snap.text.into_bytes());
        let _ = app.open_path(&path);
        RESTORE.with(|r| *r.borrow_mut() = None);
    } else if discard || r.should_close() {
        RESTORE.with(|r| *r.borrow_mut() = None);
        clear_snapshot();
    }
}

#[cfg(target_arch = "wasm32")]
fn js_ms_to_local(ms: &f64) -> String {
    let date = js_sys::Date::new(&wasm_bindgen::JsValue::from_f64(*ms));
    date.to_locale_string("en-GB", &js_sys::Object::new()).as_string().unwrap_or_else(|| format!("{ms}"))
}

#[cfg(target_arch = "wasm32")]
fn autosave_tick(app: &BimApp) {
    use bimcraft_core::time::{Duration, Instant};
    if !app.dirty() {
        return;
    }
    let due = LAST_AUTOSAVE.with(|slot| {
        let mut slot = slot.borrow_mut();
        let now = Instant::now();
        if let Some(prev) = *slot
            && now.checked_duration_since(prev).is_some_and(|d| d < Duration::from_secs(30))
        {
            return false;
        }
        *slot = Some(now);
        true
    });
    if !due {
        return;
    }
    let Ok(text) = app.session.model.to_json() else { return };
    if text.len() as u64 >= 256 << 20 {
        return;
    }
    let name = app
        .session
        .path
        .as_ref()
        .and_then(|p| p.file_name())
        .map(|n| n.to_string_lossy().into_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| format!("{}.bimcraft", app.session.model.name.replace(['/', '\\'], "-")));
    let snap = Snapshot { name, text, saved_at: js_sys::Date::now() };
    save_snapshot(snap);
}

/// MIME type for a download's extension.
#[cfg(target_arch = "wasm32")]
pub(crate) fn mime_for(ext: &str) -> &'static str {
    match ext {
        "png" => "image/png",
        "svg" => "image/svg+xml",
        "pdf" => "application/pdf",
        "glb" => "model/gltf-binary",
        "ifc" => "application/x-step",
        "dxf" => "application/dxf",
        "bimcraft" | "json" => "application/json",
        _ => "application/octet-stream",
    }
}

/// Download `bytes` as `filename`, using a writable rfd handle when the Save dialog gave one,
/// otherwise a Blob and a temporary anchor click.
#[cfg(target_arch = "wasm32")]
pub(crate) fn deliver(filename: &str, bytes: Vec<u8>, ext: &str) {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = (filename, bytes, ext);
    }
    #[cfg(target_arch = "wasm32")]
    {
        let mime = mime_for(ext);
        if let Some(handle) = take_save_handle() {
            wasm_bindgen_futures::spawn_local(async move {
                if handle.write(&bytes).await.is_err() {
                    trigger_download(&handle.file_name(), &bytes, mime);
                }
            });
            return;
        }
        trigger_download(filename, &bytes, mime);
    }
}

#[cfg(target_arch = "wasm32")]
fn trigger_download(filename: &str, bytes: &[u8], mime: &str) {
    use wasm_bindgen::JsCast;
    let Some(window) = web_sys::window() else { return };
    let Some(document) = window.document() else { return };
    let array = js_sys::Uint8Array::new_with_length(bytes.len() as u32);
    array.copy_from(bytes);
    let parts = js_sys::Array::new();
    parts.push(&array);
    let props = web_sys::BlobPropertyBag::new();
    props.set_type(mime);
    let Ok(blob) = web_sys::Blob::new_with_u8_array_sequence_and_options(&parts, &props) else { return };
    let Ok(url) = web_sys::Url::create_object_url_with_blob(&blob) else { return };
    let Ok(el) = document.create_element("a") else { return };
    let Ok(a) = el.dyn_into::<web_sys::HtmlAnchorElement>() else { return };
    a.set_href(&url);
    a.set_download(filename);
    let _ = a.set_attribute("style", "display:none");
    if let Some(body) = document.body() {
        let _ = body.append_child(&a);
        a.click();
        // Leave the URL and anchor until after the browser has started the download.
        // Revoking immediately is a common reason a programmatic <a download> never fires.
        let url = url.clone();
        let a = a.clone();
        let later = wasm_bindgen::closure::Closure::once(move || {
            a.remove();
            let _ = web_sys::Url::revoke_object_url(&url);
        });
        let _ = window.set_timeout_with_callback_and_timeout_and_arguments_0(later.as_ref().unchecked_ref(), 5_000);
        later.forget();
    }
    web_sys::console::log_1(&format!("bimcraft: download {filename} ({} bytes)", bytes.len()).into());
}

// ---- IndexedDB / localStorage recovery ----

#[cfg(target_arch = "wasm32")]
const LS_NAME: &str = "bimcraft.recovery.name";
#[cfg(target_arch = "wasm32")]
const LS_AT: &str = "bimcraft.recovery.savedAt";
#[cfg(target_arch = "wasm32")]
const LS_TEXT: &str = "bimcraft.recovery.text";
#[cfg(target_arch = "wasm32")]
const IDB_NAME: &str = "bimcraft";
#[cfg(target_arch = "wasm32")]
const IDB_STORE: &str = "recovery";
#[cfg(target_arch = "wasm32")]
const IDB_KEY: &str = "latest";
/// localStorage is typically 5 MB; stay well under that and let IndexedDB hold larger projects.
#[cfg(target_arch = "wasm32")]
const LS_LIMIT: usize = 1_500_000;

#[cfg(target_arch = "wasm32")]
fn save_snapshot(snap: Snapshot) {
    // Small projects also go in localStorage, so a restore still works if IndexedDB is blocked.
    if snap.text.len() <= LS_LIMIT {
        if let Some(ls) = local_storage() {
            let _ = ls.set_item(LS_NAME, &snap.name);
            let _ = ls.set_item(LS_AT, &snap.saved_at.to_string());
            let _ = ls.set_item(LS_TEXT, &snap.text);
        }
    } else if let Some(ls) = local_storage() {
        let _ = ls.remove_item(LS_TEXT);
        let _ = ls.set_item(LS_NAME, &snap.name);
        let _ = ls.set_item(LS_AT, &snap.saved_at.to_string());
    }
    idb_put(snap);
}

#[cfg(target_arch = "wasm32")]
fn clear_snapshot() {
    if let Some(ls) = local_storage() {
        let _ = ls.remove_item(LS_NAME);
        let _ = ls.remove_item(LS_AT);
        let _ = ls.remove_item(LS_TEXT);
    }
    idb_delete();
}

#[cfg(target_arch = "wasm32")]
fn load_snapshot(ctx: Context) {
    // Prefer IndexedDB (it holds larger projects); fall back to localStorage while it loads.
    if let Some(snap) = local_storage_snapshot() {
        RESTORE.with(|r| *r.borrow_mut() = Some(snap));
        ctx.request_repaint();
    }
    idb_get(ctx);
}

#[cfg(target_arch = "wasm32")]
fn local_storage() -> Option<web_sys::Storage> {
    web_sys::window()?.local_storage().ok().flatten()
}

#[cfg(target_arch = "wasm32")]
fn local_storage_snapshot() -> Option<Snapshot> {
    let ls = local_storage()?;
    let name = ls.get_item(LS_NAME).ok().flatten().filter(|s| !s.is_empty())?;
    let text = ls.get_item(LS_TEXT).ok().flatten().filter(|s| !s.is_empty())?;
    let saved_at = ls.get_item(LS_AT).ok().flatten().and_then(|s| s.parse().ok()).unwrap_or(0.0);
    Some(Snapshot { name, text, saved_at })
}

#[cfg(target_arch = "wasm32")]
fn open_db() -> Option<web_sys::IdbOpenDbRequest> {
    use wasm_bindgen::JsCast;
    use wasm_bindgen::closure::Closure;
    let factory = web_sys::window()?.indexed_db().ok().flatten()?;
    let req = factory.open_with_u32(IDB_NAME, 1).ok()?;
    let onup = Closure::<dyn FnMut(web_sys::Event)>::new(move |event: web_sys::Event| {
        let Some(target) = event.target() else { return };
        let Ok(req) = target.dyn_into::<web_sys::IdbOpenDbRequest>() else { return };
        let Ok(result) = req.result() else { return };
        let Ok(db) = result.dyn_into::<web_sys::IdbDatabase>() else { return };
        if !db.object_store_names().contains(IDB_STORE) {
            let _ = db.create_object_store(IDB_STORE);
        }
    });
    req.set_onupgradeneeded(Some(onup.as_ref().unchecked_ref()));
    onup.forget();
    Some(req)
}

#[cfg(target_arch = "wasm32")]
fn idb_put(snap: Snapshot) {
    use wasm_bindgen::JsCast;
    use wasm_bindgen::JsValue;
    use wasm_bindgen::closure::Closure;
    let Some(req) = open_db() else { return };
    let onok = Closure::<dyn FnMut(web_sys::Event)>::new(move |event: web_sys::Event| {
        let Some(target) = event.target() else { return };
        let Ok(req) = target.dyn_into::<web_sys::IdbOpenDbRequest>() else { return };
        let Ok(result) = req.result() else { return };
        let Ok(db) = result.dyn_into::<web_sys::IdbDatabase>() else { return };
        let Ok(tx) = db.transaction_with_str_and_mode(IDB_STORE, web_sys::IdbTransactionMode::Readwrite) else { return };
        let Ok(store) = tx.object_store(IDB_STORE) else { return };
        let obj = js_sys::Object::new();
        let _ = js_sys::Reflect::set(&obj, &JsValue::from_str("name"), &JsValue::from_str(&snap.name));
        let _ = js_sys::Reflect::set(&obj, &JsValue::from_str("text"), &JsValue::from_str(&snap.text));
        let _ = js_sys::Reflect::set(&obj, &JsValue::from_str("savedAt"), &JsValue::from_f64(snap.saved_at));
        let _ = store.put_with_key(&obj, &JsValue::from_str(IDB_KEY));
    });
    req.set_onsuccess(Some(onok.as_ref().unchecked_ref()));
    onok.forget();
}

#[cfg(target_arch = "wasm32")]
fn idb_get(ctx: Context) {
    use wasm_bindgen::JsCast;
    use wasm_bindgen::JsValue;
    use wasm_bindgen::closure::Closure;
    let Some(req) = open_db() else { return };
    let onok = Closure::<dyn FnMut(web_sys::Event)>::new(move |event: web_sys::Event| {
        let Some(target) = event.target() else { return };
        let Ok(req) = target.dyn_into::<web_sys::IdbOpenDbRequest>() else { return };
        let Ok(result) = req.result() else { return };
        let Ok(db) = result.dyn_into::<web_sys::IdbDatabase>() else { return };
        let Ok(tx) = db.transaction_with_str(IDB_STORE) else { return };
        let Ok(store) = tx.object_store(IDB_STORE) else { return };
        let Ok(get) = store.get(&JsValue::from_str(IDB_KEY)) else { return };
        let ctx = ctx.clone();
        let onget = Closure::<dyn FnMut(web_sys::Event)>::new(move |event: web_sys::Event| {
            let Some(target) = event.target() else { return };
            let Ok(req) = target.dyn_into::<web_sys::IdbRequest>() else { return };
            let Ok(value) = req.result() else { return };
            if value.is_undefined() || value.is_null() {
                return;
            }
            let name = js_sys::Reflect::get(&value, &JsValue::from_str("name")).ok().and_then(|v| v.as_string());
            let text = js_sys::Reflect::get(&value, &JsValue::from_str("text")).ok().and_then(|v| v.as_string());
            let saved_at = js_sys::Reflect::get(&value, &JsValue::from_str("savedAt")).ok().and_then(|v| v.as_f64()).unwrap_or(0.0);
            if let (Some(name), Some(text)) = (name, text) {
                if !text.is_empty() {
                    RESTORE.with(|r| *r.borrow_mut() = Some(Snapshot { name, text, saved_at }));
                    ctx.request_repaint();
                }
            }
        });
        get.set_onsuccess(Some(onget.as_ref().unchecked_ref()));
        onget.forget();
    });
    req.set_onsuccess(Some(onok.as_ref().unchecked_ref()));
    onok.forget();
}

#[cfg(target_arch = "wasm32")]
fn idb_delete() {
    use wasm_bindgen::JsCast;
    use wasm_bindgen::JsValue;
    use wasm_bindgen::closure::Closure;
    let Some(req) = open_db() else { return };
    let onok = Closure::<dyn FnMut(web_sys::Event)>::new(move |event: web_sys::Event| {
        let Some(target) = event.target() else { return };
        let Ok(req) = target.dyn_into::<web_sys::IdbOpenDbRequest>() else { return };
        let Ok(result) = req.result() else { return };
        let Ok(db) = result.dyn_into::<web_sys::IdbDatabase>() else { return };
        let Ok(tx) = db.transaction_with_str_and_mode(IDB_STORE, web_sys::IdbTransactionMode::Readwrite) else { return };
        let Ok(store) = tx.object_store(IDB_STORE) else { return };
        let _ = store.delete(&JsValue::from_str(IDB_KEY));
    });
    req.set_onsuccess(Some(onok.as_ref().unchecked_ref()));
    onok.forget();
}

// Silence unused import on native.
#[cfg(not(target_arch = "wasm32"))]
#[allow(dead_code)]
fn _unused_path_ref(p: &Path) {
    let _ = p;
}
