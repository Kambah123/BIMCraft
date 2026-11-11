//! Browser entry: the same `BimApp` in a canvas, starting on the sample house.

use bimcraft_core::engine::Session;
use bimcraft_core::sample::SAMPLE;
use bimcraft_ui_egui::{BimApp, lang};
use serde_json::json;
use wasm_bindgen::JsCast;

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
        // Leak the runner so it outlives this future. Event handlers hold Rc clones of
        // it too, but dropping the last owner would call through to the wgpu instance.
        let runner = Box::leak(Box::new(eframe::WebRunner::new()));
        match runner.start(canvas, eframe::WebOptions::default(), Box::new(move |cc| Ok(Box::new(app(cc))))).await {
            Ok(()) => web_sys::console::log_1(&"bimcraft: eframe started".into()),
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
