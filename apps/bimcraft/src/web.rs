//! Browser entry: the same `BimApp` in a canvas, starting on the sample house.

use bimcraft_core::engine::Session;
use bimcraft_core::sample::SAMPLE;
use bimcraft_ui_egui::{BimApp, lang, web_files};
use serde_json::json;
use wasm_bindgen::JsCast;
use wasm_bindgen::JsValue;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::prelude::wasm_bindgen;

/// Starts the window in `#the_canvas_id` and loads the sample house (there is no command line on the web).
pub fn start() {
    // Say panics in the console before eframe's runner is up: building the sample is done
    // in the start callback, but anything that fails earlier should still be visible.
    std::panic::set_hook(Box::new(|info| {
        web_sys::console::error_1(&format!("bimcraft panic: {info}").into());
        fail(&format!("the app stopped while starting ({info})"));
    }));

    wasm_bindgen_futures::spawn_local(async move {
        let Some(window) = web_sys::window() else {
            web_sys::console::error_1(&"bimcraft: no window".into());
            return;
        };
        let Some(document) = window.document() else {
            web_sys::console::error_1(&"bimcraft: no document".into());
            return;
        };
        let Some(element) = document.get_element_by_id("the_canvas_id") else {
            web_sys::console::error_1(&"bimcraft: missing canvas #the_canvas_id".into());
            return;
        };
        let Ok(canvas) = element.dyn_into::<web_sys::HtmlCanvasElement>() else {
            web_sys::console::error_1(&"bimcraft: #the_canvas_id is not a canvas".into());
            return;
        };
        install_hooks(&window, &document);
        // The browser's graphics, WebGPU first where there is a working one, then WebGL2; `?gfx=gl` asks for WebGL2 only.
        // A start that fails is tried again on WebGL2 alone, on a fresh canvas (a canvas keeps the first context it gave).
        let gl_only = window.location().search().ok().is_some_and(|q| q.contains("gfx=gl"));
        let mut canvas = canvas;
        let mut tries: Vec<bool> = if gl_only { vec![true] } else { vec![false, true] };
        tries.reverse();
        while let Some(gl) = tries.pop() {
            // Leak the runner so it outlives this future. Event handlers hold Rc clones of
            // it too, but dropping the last owner would call through to the wgpu instance.
            let runner = Box::leak(Box::new(eframe::WebRunner::new()));
            match runner.start(canvas.clone(), web_options(gl), Box::new(move |cc| Ok(Box::new(app(cc))))).await {
                Ok(()) => {
                    web_sys::console::log_1(&format!("bimcraft: eframe started{}", if gl { " (WebGL2)" } else { "" }).into());
                    let _ = js_sys::Reflect::set(&window, &JsValue::from_str("__bimcraftReady"), &JsValue::TRUE);
                    // CampusBIM's loading screen (index.html) gives way to the app.
                    if let Some(el) = document.get_element_by_id("campusbim-loading") {
                        el.remove();
                    }
                    return;
                }
                Err(err) => {
                    let text = err.as_string().unwrap_or_else(|| format!("{err:?}"));
                    web_sys::console::error_1(
                        &format!("bimcraft: the graphics didn't start{}: {text}", if gl { " on WebGL2" } else { "" }).into(),
                    );
                    if tries.is_empty() {
                        fail(&text);
                        return;
                    }
                    match fresh_canvas(&document, &canvas) {
                        Some(next) => canvas = next,
                        None => {
                            fail(&text);
                            return;
                        }
                    }
                }
            }
        }
    });
}

/// eframe's options, with a device that never asks for more than the browser's card has: egui asks for 8192 px textures,
/// which many phones' WebGL2 (and some laptops') don't give, and the start then failed with the loading screen up.
fn web_options(gl_only: bool) -> eframe::WebOptions {
    let mut options = eframe::WebOptions::default();
    if let eframe::egui_wgpu::WgpuSetup::CreateNew(setup) = &mut options.wgpu_options.wgpu_setup {
        if gl_only {
            setup.instance_descriptor.backends = wgpu::Backends::GL;
        }
        setup.device_descriptor = std::sync::Arc::new(|adapter: &wgpu::Adapter| {
            let has = adapter.limits();
            let base = if adapter.get_info().backend == wgpu::Backend::Gl {
                wgpu::Limits::downlevel_webgl2_defaults()
            } else {
                wgpu::Limits::default()
            };
            let side = 8192.min(has.max_texture_dimension_2d);
            let required_limits = wgpu::Limits {
                max_texture_dimension_1d: base.max_texture_dimension_1d.min(has.max_texture_dimension_1d),
                max_texture_dimension_2d: side,
                max_texture_dimension_3d: base.max_texture_dimension_3d.min(has.max_texture_dimension_3d),
                ..base
            };
            wgpu::DeviceDescriptor { label: Some("egui wgpu device"), required_limits, ..Default::default() }
        });
    }
    options
}

/// A new canvas in the old one's place, with its id, for a second try with other graphics.
fn fresh_canvas(document: &web_sys::Document, old: &web_sys::HtmlCanvasElement) -> Option<web_sys::HtmlCanvasElement> {
    let next = document.create_element("canvas").ok()?.dyn_into::<web_sys::HtmlCanvasElement>().ok()?;
    next.set_id("the_canvas_id");
    old.replace_with_with_node_1(&next).ok()?;
    Some(next)
}

/// Says on the loading screen that the app couldn't start, with a way to try again (`campusbimFail` in index.html).
fn fail(detail: &str) {
    let Some(window) = web_sys::window() else { return };
    if js_sys::Reflect::get(&window, &JsValue::from_str("__bimcraftReady")).ok().is_some_and(|v| v.is_truthy()) {
        return;
    }
    if let Ok(f) = js_sys::Reflect::get(&window, &JsValue::from_str("campusbimFail"))
        && let Ok(f) = f.dyn_into::<js_sys::Function>()
    {
        let _ = f.call1(&JsValue::NULL, &JsValue::from_str(detail));
    }
}

fn app(cc: &eframe::CreationContext<'_>) -> BimApp {
    let language = lang::from_env();
    let mut session = Session::new();
    let _ = session.execute("new", &json!({ "language": language.project().code() }));
    let _ = session.script(SAMPLE);
    let (model, path) = ((*session.model).clone(), session.path.clone());
    session.reset(model, path);
    let mut app = BimApp::new(session);
    app.lang = language;
    app.own_title_bar = true;
    // CampusBIM Dark, unless the user picked another theme in this browser before.
    bimcraft_ui_egui::brand::first_look(&mut app);
    if let Some(state) = cc.wgpu_render_state.as_ref() {
        app.use_graphics_card(state);
    }
    // The 3D view is drawn on the processor by default: on some real GPUs (Chrome on macOS, WebGL2 through ANGLE/Metal)
    // the card's path froze the tab. `?3d=gpu` opts in to the card, to test it; `?3d=cpu` is still understood.
    let query = web_sys::window().and_then(|w| w.location().search().ok()).unwrap_or_default();
    if !query.contains("3d=gpu") {
        app.draw_3d_on_the_processor();
    }
    app
}

/// Hidden file input plus `window.__bimcraftQueueFile` / `__bimcraftExport` for tests and scripted I/O.
fn install_hooks(window: &web_sys::Window, document: &web_sys::Document) {
    if let Some(el) = document.get_element_by_id("bimcraft-file-input") {
        if let Ok(input) = el.dyn_into::<web_sys::HtmlInputElement>() {
            let input_el = input.clone();
            let onchange = Closure::<dyn FnMut()>::new(move || {
                let Some(files) = input_el.files() else { return };
                for i in 0..files.length() {
                    let Some(file) = files.item(i) else { continue };
                    let name = file.name();
                    wasm_bindgen_futures::spawn_local(async move {
                        match wasm_bindgen_futures::JsFuture::from(file.array_buffer()).await {
                            Ok(buf) => {
                                let data = js_sys::Uint8Array::new(&buf);
                                web_files::queue_file(name, data.to_vec());
                                wake_canvas();
                            }
                            Err(e) => web_sys::console::error_1(&e),
                        }
                    });
                }
                input_el.set_value("");
            });
            let _ = input.add_event_listener_with_callback("change", onchange.as_ref().unchecked_ref());
            onchange.forget();
        }
    }

    let queue = Closure::<dyn FnMut(String, js_sys::Uint8Array)>::new(move |name: String, data: js_sys::Uint8Array| {
        web_files::queue_file(name, data.to_vec());
        wake_canvas();
    });
    let _ = js_sys::Reflect::set(window, &JsValue::from_str("__bimcraftQueueFile"), queue.as_ref().unchecked_ref());
    queue.forget();

    let export = Closure::<dyn FnMut(String)>::new(move |ext: String| {
        web_files::queue_export(ext);
        wake_canvas();
    });
    let _ = js_sys::Reflect::set(window, &JsValue::from_str("__bimcraftExport"), export.as_ref().unchecked_ref());
    export.forget();
}

/// The File API and the test hooks run outside egui's event loop; poke the canvas so a frame runs.
fn wake_canvas() {
    if let Some(document) = web_sys::window().and_then(|w| w.document()) {
        if let Some(el) = document.get_element_by_id("the_canvas_id") {
            let _ = el.dispatch_event(
                &web_sys::Event::new("pointermove").unwrap_or_else(|_| web_sys::Event::new("resize").unwrap_or(js_dummy())),
            );
        }
    }
    if let Some(w) = web_sys::window() {
        let _ = w.request_animation_frame(&js_sys::Function::new_no_args(""));
    }
}

fn js_dummy() -> web_sys::Event {
    // Event::new only fails if the type name is illegal; "ping" is fine.
    web_sys::Event::new("ping").expect("Event")
}

/// Kept so wasm-bindgen names the exports a debugger can call directly too.
#[wasm_bindgen]
pub fn bimcraft_queue_file(name: String, data: js_sys::Uint8Array) {
    web_files::queue_file(name, data.to_vec());
}

#[wasm_bindgen]
pub fn bimcraft_queue_export(ext: String) {
    web_files::queue_export(ext);
}
