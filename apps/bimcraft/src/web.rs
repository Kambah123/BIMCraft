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
        // Leak the runner so it outlives this future. Event handlers hold Rc clones of
        // it too, but dropping the last owner would call through to the wgpu instance.
        let runner = Box::leak(Box::new(eframe::WebRunner::new()));
        match runner.start(canvas, eframe::WebOptions::default(), Box::new(move |cc| Ok(Box::new(app(cc))))).await {
            Ok(()) => {
                web_sys::console::log_1(&"bimcraft: eframe started".into());
                let _ = js_sys::Reflect::set(&window, &JsValue::from_str("__bimcraftReady"), &JsValue::TRUE);
            }
            Err(err) => web_sys::console::error_1(&err),
        }
    });
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
    if let Some(state) = cc.wgpu_render_state.as_ref() {
        app.use_graphics_card(state);
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
