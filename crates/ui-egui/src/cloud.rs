//! CampusBIM accounts and cloud save, in the browser only: sign up, sign in and sign out with an email and a password,
//! Save to Cloud and My Projects. It talks to Supabase's Auth, REST and Storage HTTP APIs with the page's own `fetch`;
//! the session (its access and refresh tokens) is kept in `localStorage` and refreshed before it runs out.
//!
//! The project URL and the publishable key are public (Row Level Security keeps each user to their own rows and files),
//! so they are built in: `CAMPUSBIM_SUPABASE_URL` and `CAMPUSBIM_SUPABASE_KEY` at build time replace the defaults.

use std::cell::RefCell;
use std::path::PathBuf;
use std::sync::Arc;

use bimcraft_core::model::Model;
use egui::{Align, Context, Id, Key, Layout, RichText, ScrollArea, TextEdit, Ui};
use serde_json::{Value, json};
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;

use crate::lang::t;
use crate::style::pal;
use crate::{BimApp, Status};

const DEFAULT_URL: &str = "https://crqrtwofevttrkfvkdpp.supabase.co";
const DEFAULT_KEY: &str = "sb_publishable_14607Cwr-kBKs0StEhI9bg_WID2MBk0";
const BUCKET: &str = "projects";
const LS_AUTH: &str = "campusbim.auth";
/// Larger than this is refused before it is sent (the bucket's own limit is 100 MB).
const MAX_UPLOAD: usize = 100 << 20;

fn base_url() -> &'static str {
    option_env!("CAMPUSBIM_SUPABASE_URL").filter(|s| !s.is_empty()).unwrap_or(DEFAULT_URL).trim_end_matches('/')
}

fn api_key() -> &'static str {
    option_env!("CAMPUSBIM_SUPABASE_KEY").filter(|s| !s.is_empty()).unwrap_or(DEFAULT_KEY)
}

#[derive(Clone, Default, PartialEq)]
struct Auth {
    access: String,
    refresh: String,
    /// Seconds since 1970.
    expires_at: f64,
    user: String,
    email: String,
}

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    SignIn,
    SignUp,
}

struct AccountDialog {
    email: String,
    password: String,
    mode: Mode,
    busy: bool,
    error: Option<String>,
    info: Option<String>,
}

#[derive(Clone)]
struct Row {
    id: String,
    name: String,
    updated_at: String,
    size: i64,
    path: String,
}

#[derive(Default)]
struct ProjectsDialog {
    rows: Option<Vec<Row>>,
    busy: bool,
    error: Option<String>,
    confirm: Option<String>,
}

enum Event {
    /// `None`: the account was made but has to be confirmed by email first.
    SignedIn(Result<Option<Auth>, String>),
    Refreshed(Result<Auth, String>),
    Saved(Result<(String, String, usize), String>),
    Listed(Result<Vec<Row>, String>),
    Opened(Result<(String, String, Vec<u8>), String>),
    Deleted(Result<String, String>),
}

#[derive(Default)]
struct Cloud {
    auth: Option<Auth>,
    loaded: bool,
    account: Option<AccountDialog>,
    projects: Option<ProjectsDialog>,
    /// The cloud project on screen (its id and name) and the window's project it belongs to.
    current: Option<(String, String, u64)>,
    refreshing: bool,
    saving: Option<Arc<Model>>,
    save_after_sign_in: bool,
    events: Vec<Event>,
    message: String,
    published: String,
}

thread_local! {
    static CLOUD: RefCell<Cloud> = RefCell::new(Cloud::default());
    static REPAINT: RefCell<Option<Context>> = const { RefCell::new(None) };
}

fn with<R>(f: impl FnOnce(&mut Cloud) -> R) -> R {
    CLOUD.with(|c| f(&mut c.borrow_mut()))
}

fn push(event: Event) {
    with(|c| c.events.push(event));
    REPAINT.with(|r| {
        if let Some(ctx) = r.borrow().as_ref() {
            ctx.request_repaint();
        }
    });
}

fn now_secs() -> f64 {
    js_sys::Date::now() / 1000.0
}

// ---------- HTTP ----------

enum Body {
    Json(Value),
    Bytes(Vec<u8>),
}

fn js_err(e: JsValue) -> String {
    e.as_string()
        .or_else(|| js_sys::Reflect::get(&e, &JsValue::from_str("message")).ok().and_then(|m| m.as_string()))
        .unwrap_or_else(|| "the browser refused the request".to_owned())
}

async fn http(method: &str, url: &str, headers: &[(&str, String)], body: Option<Body>) -> Result<(u16, Vec<u8>), String> {
    let init = web_sys::RequestInit::new();
    init.set_method(method);
    let h = web_sys::Headers::new().map_err(js_err)?;
    h.set("apikey", api_key()).map_err(js_err)?;
    for (k, v) in headers {
        h.set(k, v).map_err(js_err)?;
    }
    match body {
        Some(Body::Json(v)) => {
            h.set("Content-Type", "application/json").map_err(js_err)?;
            init.set_body(&JsValue::from_str(&v.to_string()));
        }
        Some(Body::Bytes(b)) => {
            let array = js_sys::Uint8Array::new_with_length(b.len() as u32);
            array.copy_from(&b);
            init.set_body(&array);
        }
        None => {}
    }
    init.set_headers(&h);
    let request = web_sys::Request::new_with_str_and_init(url, &init).map_err(js_err)?;
    let window = web_sys::window().ok_or_else(|| "no window".to_owned())?;
    let response = JsFuture::from(window.fetch_with_request(&request))
        .await
        .map_err(|_| t("Couldn't reach the CampusBIM cloud. Check the connection and try again.").to_owned())?;
    let response: web_sys::Response = response.dyn_into().map_err(js_err)?;
    let status = response.status();
    let buffer = JsFuture::from(response.array_buffer().map_err(js_err)?).await.map_err(js_err)?;
    Ok((status, js_sys::Uint8Array::new(&buffer).to_vec()))
}

/// The error Supabase gave, in its words, or the status.
fn error_text(status: u16, bytes: &[u8]) -> String {
    let v: Value = serde_json::from_slice(bytes).unwrap_or(Value::Null);
    for key in ["msg", "error_description", "message", "error"] {
        if let Some(s) = v.get(key).and_then(Value::as_str)
            && !s.is_empty()
        {
            let mut s = s.to_owned();
            if let Some(first) = s.get(..1) {
                s = first.to_uppercase() + s.get(1..).unwrap_or("");
            }
            return s;
        }
    }
    format!("The cloud answered {status}.")
}

async fn json_call(method: &str, url: &str, headers: &[(&str, String)], body: Option<Body>) -> Result<Value, String> {
    let (status, bytes) = http(method, url, headers, body).await?;
    if !(200..300).contains(&status) {
        return Err(error_text(status, &bytes));
    }
    if bytes.is_empty() {
        return Ok(Value::Null);
    }
    serde_json::from_slice(&bytes).map_err(|e| format!("The cloud's answer couldn't be read: {e}"))
}

fn parse_auth(v: &Value) -> Option<Auth> {
    let access = v.get("access_token")?.as_str()?.to_owned();
    let refresh = v.get("refresh_token")?.as_str()?.to_owned();
    let expires_at = v
        .get("expires_at")
        .and_then(Value::as_f64)
        .or_else(|| v.get("expires_in").and_then(Value::as_f64).map(|s| now_secs() + s))
        .unwrap_or_else(|| now_secs() + 3600.0);
    let user = v.get("user")?;
    Some(Auth {
        access,
        refresh,
        expires_at,
        user: user.get("id")?.as_str()?.to_owned(),
        email: user.get("email").and_then(Value::as_str).unwrap_or("").to_owned(),
    })
}

async fn sign_in(email: String, password: String, mode: Mode) -> Result<Option<Auth>, String> {
    let url = match mode {
        Mode::SignIn => format!("{}/auth/v1/token?grant_type=password", base_url()),
        Mode::SignUp => format!("{}/auth/v1/signup", base_url()),
    };
    let v = json_call("POST", &url, &[], Some(Body::Json(json!({"email": email, "password": password})))).await?;
    match parse_auth(&v) {
        Some(a) => Ok(Some(a)),
        None if mode == Mode::SignUp => Ok(None),
        None => Err(t("The cloud didn't send a session back.").to_owned()),
    }
}

async fn refresh(token: String) -> Result<Auth, String> {
    let url = format!("{}/auth/v1/token?grant_type=refresh_token", base_url());
    let v = json_call("POST", &url, &[], Some(Body::Json(json!({"refresh_token": token})))).await?;
    parse_auth(&v).ok_or_else(|| t("The cloud didn't send a session back.").to_owned())
}

/// The access token, refreshed first when it runs out within a minute.
async fn bearer() -> Result<(String, String), String> {
    let auth = with(|c| c.auth.clone()).ok_or_else(|| t("Sign in first.").to_owned())?;
    if auth.expires_at - now_secs() > 60.0 {
        return Ok((format!("Bearer {}", auth.access), auth.user));
    }
    match refresh(auth.refresh.clone()).await {
        Ok(fresh) => {
            let out = (format!("Bearer {}", fresh.access), fresh.user.clone());
            with(|c| c.auth = Some(fresh.clone()));
            store_auth(Some(&fresh));
            Ok(out)
        }
        Err(e) => {
            with(|c| c.auth = None);
            store_auth(None);
            Err(format!("{} {e}", t("Your session ran out; sign in again.")))
        }
    }
}

fn encode_path(path: &str) -> String {
    path.split('/').map(|p| String::from(js_sys::encode_uri_component(p))).collect::<Vec<_>>().join("/")
}

async fn upload(id: String, name: String, bytes: Vec<u8>) -> Result<(String, String, usize), String> {
    let (bearer, user) = bearer().await?;
    let path = format!("{user}/{id}.bimcraft");
    let size = bytes.len();
    let url = format!("{}/storage/v1/object/{BUCKET}/{}", base_url(), encode_path(&path));
    json_call(
        "POST",
        &url,
        &[("Authorization", bearer.clone()), ("x-upsert", "true".into()), ("Content-Type", "application/json".into())],
        Some(Body::Bytes(bytes)),
    )
    .await?;
    let url = format!("{}/rest/v1/projects?on_conflict=id", base_url());
    json_call(
        "POST",
        &url,
        &[("Authorization", bearer), ("Prefer", "resolution=merge-duplicates,return=minimal".into())],
        Some(Body::Json(json!({"id": id, "name": name, "size_bytes": size, "storage_path": path}))),
    )
    .await?;
    Ok((id, name, size))
}

async fn list() -> Result<Vec<Row>, String> {
    let (bearer, _) = bearer().await?;
    let url = format!("{}/rest/v1/projects?select=id,name,updated_at,size_bytes,storage_path&order=updated_at.desc", base_url());
    let v = json_call("GET", &url, &[("Authorization", bearer)], None).await?;
    let rows = v.as_array().cloned().unwrap_or_default();
    Ok(rows
        .iter()
        .filter_map(|r| {
            Some(Row {
                id: r.get("id")?.as_str()?.to_owned(),
                name: r.get("name")?.as_str()?.to_owned(),
                updated_at: r.get("updated_at").and_then(Value::as_str).unwrap_or("").to_owned(),
                size: r.get("size_bytes").and_then(Value::as_i64).unwrap_or(0),
                path: r.get("storage_path")?.as_str()?.to_owned(),
            })
        })
        .collect())
}

async fn download(row: Row) -> Result<(String, String, Vec<u8>), String> {
    let (bearer, _) = bearer().await?;
    let url = format!("{}/storage/v1/object/authenticated/{BUCKET}/{}", base_url(), encode_path(&row.path));
    let (status, bytes) = http("GET", &url, &[("Authorization", bearer)], None).await?;
    if !(200..300).contains(&status) {
        return Err(error_text(status, &bytes));
    }
    Ok((row.id, row.name, bytes))
}

async fn delete(row: Row) -> Result<String, String> {
    let (bearer, _) = bearer().await?;
    let url = format!("{}/storage/v1/object/{BUCKET}/{}", base_url(), encode_path(&row.path));
    let (status, bytes) = http("DELETE", &url, &[("Authorization", bearer.clone())], None).await?;
    // A row whose file is already gone can still be deleted.
    if !(200..300).contains(&status) && status != 404 && status != 400 {
        return Err(error_text(status, &bytes));
    }
    let url = format!("{}/rest/v1/projects?id=eq.{}", base_url(), js_sys::encode_uri_component(&row.id));
    json_call("DELETE", &url, &[("Authorization", bearer)], None).await?;
    Ok(row.id)
}

// ---------- the session in localStorage ----------

fn local_storage() -> Option<web_sys::Storage> {
    web_sys::window()?.local_storage().ok().flatten()
}

fn store_auth(auth: Option<&Auth>) {
    let Some(ls) = local_storage() else { return };
    match auth {
        Some(a) => {
            let v = json!({"access_token": a.access, "refresh_token": a.refresh, "expires_at": a.expires_at, "user": {"id": a.user, "email": a.email}});
            let _ = ls.set_item(LS_AUTH, &v.to_string());
        }
        None => {
            let _ = ls.remove_item(LS_AUTH);
        }
    }
}

fn load_auth() -> Option<Auth> {
    let text = local_storage()?.get_item(LS_AUTH).ok().flatten()?;
    parse_auth(&serde_json::from_str(&text).ok()?)
}

// ---------- actions ----------

fn spawn_sign_in(email: String, password: String, mode: Mode) {
    wasm_bindgen_futures::spawn_local(async move { push(Event::SignedIn(sign_in(email, password, mode).await)) });
}

fn spawn_list() {
    with(|c| {
        if let Some(p) = c.projects.as_mut() {
            p.busy = true;
            p.error = None;
        }
    });
    wasm_bindgen_futures::spawn_local(async move { push(Event::Listed(list().await)) });
}

fn set_message(app: &mut BimApp, text: String, error: bool) {
    with(|c| c.message.clone_from(&text));
    app.status = Status { text, error };
}

/// File > Save to Cloud: the project as a `.bimcraft`, uploaded under the user's folder, its row added or brought up to date.
pub(crate) fn save_to_cloud(app: &mut BimApp) {
    if with(|c| c.saving.is_some()) {
        return;
    }
    if with(|c| c.auth.is_none()) {
        with(|c| c.save_after_sign_in = true);
        open_account(Mode::SignIn);
        return;
    }
    let bytes = match app.session.encode("bimcraft", &json!({})) {
        Ok((bytes, _)) => bytes,
        Err(e) => return set_message(app, e, true),
    };
    if bytes.len() > MAX_UPLOAD {
        return set_message(app, t("This project is larger than 100 MB, the most the cloud keeps for one project.").to_owned(), true);
    }
    let uid = app.session.model.uid;
    let name = app.session.model.name.trim().to_owned();
    let name = if name.is_empty() { "Untitled".to_owned() } else { name.chars().take(200).collect() };
    let id = with(|c| c.current.as_ref().filter(|(_, _, u)| *u == uid).map(|(id, _, _)| id.clone())).unwrap_or_else(new_id);
    with(|c| c.saving = Some(Arc::clone(&app.session.model)));
    set_message(app, format!("{} {name}…", t("Saving to the cloud:")), false);
    wasm_bindgen_futures::spawn_local(async move { push(Event::Saved(upload(id, name, bytes).await)) });
}

fn new_id() -> String {
    let crypto = web_sys::window().and_then(|w| js_sys::Reflect::get(&w, &JsValue::from_str("crypto")).ok());
    let made = crypto.and_then(|c| {
        let f = js_sys::Reflect::get(&c, &JsValue::from_str("randomUUID")).ok()?.dyn_into::<js_sys::Function>().ok()?;
        f.call0(&c).ok()?.as_string()
    });
    made.unwrap_or_else(|| {
        let r = || (js_sys::Math::random() * 4_294_967_296.0) as u32;
        let (a, b, c, d) = (r(), r(), r(), r());
        format!("{a:08x}-{:04x}-4{:03x}-{:04x}-{:04x}{d:08x}", b >> 16, b & 0xfff, (c >> 16 & 0x3fff) | 0x8000, c & 0xffff)
    })
}

fn open_account(mode: Mode) {
    with(|c| {
        let email = c.account.as_ref().map(|a| a.email.clone()).unwrap_or_default();
        c.account = Some(AccountDialog { email, password: String::new(), mode, busy: false, error: None, info: None });
    });
}

pub(crate) fn open_projects() {
    if with(|c| c.auth.is_none()) {
        open_account(Mode::SignIn);
        return;
    }
    with(|c| c.projects = Some(ProjectsDialog::default()));
    spawn_list();
}

pub(crate) fn sign_out(app: &mut BimApp) {
    let auth = with(|c| {
        c.projects = None;
        c.current = None;
        c.auth.take()
    });
    store_auth(None);
    if let Some(a) = auth {
        let bearer = format!("Bearer {}", a.access);
        wasm_bindgen_futures::spawn_local(async move {
            let url = format!("{}/auth/v1/logout", base_url());
            let _ = http("POST", &url, &[("Authorization", bearer)], None).await;
        });
    }
    set_message(app, t("Signed out of CampusBIM.").to_owned(), false);
}

pub(crate) fn signed_in_email() -> Option<String> {
    with(|c| c.auth.as_ref().map(|a| a.email.clone()))
}

// ---------- each frame ----------

/// Load the session once, refresh it before it runs out, apply what came back from the network, and draw the dialogs.
pub(crate) fn poll(app: &mut BimApp, ctx: &Context) {
    REPAINT.with(|r| *r.borrow_mut() = Some(ctx.clone()));
    if !with(|c| std::mem::replace(&mut c.loaded, true)) {
        let auth = load_auth();
        with(|c| c.auth = auth);
    }
    if let Some(token) = with(|c| match &c.auth {
        Some(a) if !c.refreshing && a.expires_at - now_secs() < 300.0 => {
            c.refreshing = true;
            Some(a.refresh.clone())
        }
        _ => None,
    }) {
        wasm_bindgen_futures::spawn_local(async move { push(Event::Refreshed(refresh(token).await)) });
    }
    if let Some(a) = with(|c| c.auth.as_ref().map(|a| a.expires_at)) {
        let left = (a - now_secs() - 290.0).clamp(1.0, 3600.0);
        ctx.request_repaint_after(std::time::Duration::from_secs_f64(left));
    }
    for event in with(|c| std::mem::take(&mut c.events)) {
        apply(app, event);
    }
    let uid = app.session.model.uid;
    with(|c| {
        if c.current.as_ref().is_some_and(|(_, _, u)| *u != uid) {
            c.current = None;
        }
    });
    account_window(app, ctx);
    projects_window(app, ctx);
    publish();
}

fn apply(app: &mut BimApp, event: Event) {
    match event {
        Event::SignedIn(Ok(Some(auth))) => {
            store_auth(Some(&auth));
            let email = auth.email.clone();
            let then_save = with(|c| {
                c.auth = Some(auth);
                c.account = None;
                std::mem::take(&mut c.save_after_sign_in)
            });
            set_message(app, format!("{} {email}", t("Signed in as")), false);
            if then_save {
                save_to_cloud(app);
            }
        }
        Event::SignedIn(Ok(None)) => with(|c| {
            if let Some(a) = c.account.as_mut() {
                a.busy = false;
                a.mode = Mode::SignIn;
                a.password.clear();
                a.info = Some(t("Check your email to confirm your account, then sign in.").to_owned());
            }
        }),
        Event::SignedIn(Err(e)) => with(|c| {
            if let Some(a) = c.account.as_mut() {
                a.busy = false;
                a.error = Some(e);
            }
        }),
        Event::Refreshed(result) => {
            let failed = with(|c| {
                c.refreshing = false;
                match result {
                    Ok(a) => {
                        store_auth(Some(&a));
                        c.auth = Some(a);
                        false
                    }
                    // Signed out elsewhere, or the refresh token is spent: a network error keeps the session to try again.
                    Err(e) if !e.starts_with(t("Couldn't reach")) => {
                        c.auth = None;
                        store_auth(None);
                        true
                    }
                    Err(_) => {
                        if let Some(a) = c.auth.as_mut() {
                            a.expires_at = a.expires_at.max(now_secs() + 330.0);
                        }
                        false
                    }
                }
            });
            if failed {
                set_message(app, t("Your CampusBIM session ran out; sign in again.").to_owned(), true);
            }
        }
        Event::Saved(Ok((id, name, size))) => {
            let saved = with(|c| c.saving.take());
            if let Some(model) = saved
                && model.uid == app.session.model.uid
            {
                app.saved = model;
            }
            let uid = app.session.model.uid;
            with(|c| c.current = Some((id, name.clone(), uid)));
            set_message(app, format!("{} {name} ({})", t("Saved to the cloud:"), size_text(size as i64)), false);
            if with(|c| c.projects.is_some()) {
                spawn_list();
            }
        }
        Event::Saved(Err(e)) => {
            with(|c| c.saving = None);
            set_message(app, format!("{} {e}", t("Couldn't save to the cloud.")), true);
        }
        Event::Listed(result) => with(|c| {
            if let Some(p) = c.projects.as_mut() {
                p.busy = false;
                match result {
                    Ok(rows) => p.rows = Some(rows),
                    Err(e) => p.error = Some(e),
                }
            }
        }),
        Event::Opened(Ok((id, name, bytes))) => {
            let file = format!("{}.bimcraft", name.replace(['/', '\\'], "-"));
            let path = PathBuf::from(&file);
            app.session.provide_bytes(&path, bytes);
            if app.open_path(&path) {
                let uid = app.session.model.uid;
                with(|c| {
                    c.current = Some((id, name.clone(), uid));
                    c.projects = None;
                });
                set_message(app, format!("{} {name}", t("Opened from the cloud:")), false);
            } else {
                let e = app.status.text.clone();
                with(|c| {
                    if let Some(p) = c.projects.as_mut() {
                        p.busy = false;
                        p.error = Some(e);
                    }
                });
            }
        }
        Event::Opened(Err(e)) | Event::Deleted(Err(e)) => with(|c| {
            if let Some(p) = c.projects.as_mut() {
                p.busy = false;
                p.error = Some(e);
            }
        }),
        Event::Deleted(Ok(id)) => {
            with(|c| {
                if c.current.as_ref().is_some_and(|(cur, _, _)| *cur == id) {
                    c.current = None;
                }
                if let Some(p) = c.projects.as_mut() {
                    p.busy = false;
                    if let Some(rows) = p.rows.as_mut() {
                        rows.retain(|r| r.id != id);
                    }
                }
            });
            set_message(app, t("Deleted from the cloud.").to_owned(), false);
        }
    }
}

/// `window.__bimcraftCloud`: the account and the cloud projects as JSON, for tests and page scripts (never the tokens).
fn publish() {
    let text = with(|c| {
        json!({
            "signedIn": c.auth.is_some(),
            "email": c.auth.as_ref().map(|a| a.email.clone()),
            "current": c.current.as_ref().map(|(id, name, _)| json!({"id": id, "name": name})),
            "saving": c.saving.is_some(),
            "accountOpen": c.account.is_some(),
            "accountError": c.account.as_ref().and_then(|a| a.error.clone()),
            "accountInfo": c.account.as_ref().and_then(|a| a.info.clone()),
            "projects": c.projects.as_ref().and_then(|p| p.rows.as_ref()).map(|rows| rows.iter().map(|r| json!({"id": r.id, "name": r.name, "size": r.size})).collect::<Vec<_>>()),
            "projectsError": c.projects.as_ref().and_then(|p| p.error.clone()),
            "message": c.message,
        })
        .to_string()
    });
    let changed = with(|c| {
        if c.published == text {
            false
        } else {
            c.published.clone_from(&text);
            true
        }
    });
    if changed && let Some(window) = web_sys::window() {
        let _ = js_sys::Reflect::set(&window, &JsValue::from_str("__bimcraftCloud"), &JsValue::from_str(&text));
    }
}

fn size_text(bytes: i64) -> String {
    if bytes >= 1 << 20 { format!("{:.1} MB", bytes as f64 / 1_048_576.0) } else { format!("{} KB", (bytes + 1023) / 1024) }
}

fn when_text(iso: &str) -> String {
    let date = js_sys::Date::new(&JsValue::from_str(iso));
    if date.get_time().is_nan() {
        return iso.to_owned();
    }
    let opts = js_sys::Object::new();
    for (k, v) in [("dateStyle", "medium"), ("timeStyle", "short")] {
        let _ = js_sys::Reflect::set(&opts, &JsValue::from_str(k), &JsValue::from_str(v));
    }
    date.to_locale_string("en-GB", &opts).as_string().unwrap_or_else(|| iso.to_owned())
}

// ---------- the dialogs ----------

fn account_window(_app: &mut BimApp, ctx: &Context) {
    let Some(mut d) = with(|c| c.account.take()) else { return };
    let mut close = false;
    let mut submit = false;
    let title = t("CampusBIM account");
    let r = crate::popup::Modal::new(Id::new("cloud-account"), title).width(400.0).show(ctx, |ui| {
        crate::popup::body(ui, |ui| {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.add_enabled_ui(!d.busy, |ui| {
                    if ui.selectable_label(d.mode == Mode::SignIn, t("Sign in")).clicked() {
                        d.mode = Mode::SignIn;
                        d.error = None;
                    }
                    if ui.selectable_label(d.mode == Mode::SignUp, t("Create account")).clicked() {
                        d.mode = Mode::SignUp;
                        d.error = None;
                        d.info = None;
                    }
                });
            });
            ui.add_space(8.0);
            ui.label(
                RichText::new(t(match d.mode {
                    Mode::SignIn => "Sign in to save projects to the cloud and open them on any computer.",
                    Mode::SignUp => "Make a free account with your email and a password of at least 6 characters.",
                }))
                .color(pal().muted),
            );
            ui.add_space(8.0);
            ui.label(t("Email"));
            let email =
                ui.add_enabled(!d.busy, TextEdit::singleline(&mut d.email).hint_text("you@school.edu.ng").desired_width(f32::INFINITY));
            ui.add_space(4.0);
            ui.label(t("Password"));
            let pass = ui.add_enabled(!d.busy, TextEdit::singleline(&mut d.password).password(true).desired_width(f32::INFINITY));
            if (email.lost_focus() || pass.lost_focus()) && ui.input(|i| i.key_pressed(Key::Enter)) {
                submit = true;
            }
            if let Some(e) = &d.error {
                ui.add_space(6.0);
                ui.colored_label(pal().error, e);
            }
            if let Some(i) = &d.info {
                ui.add_space(6.0);
                ui.colored_label(pal().accent, i);
            }
            if d.busy {
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(t("Talking to the cloud…"));
                });
            }
        });
        crate::popup::footer(ui, |ui| {
            ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                let label = t(match d.mode {
                    Mode::SignIn => "Sign in",
                    Mode::SignUp => "Create account",
                });
                if ui.add_enabled(!d.busy, egui::Button::new(RichText::new(label).strong())).clicked() {
                    submit = true;
                }
                if ui.button(t("Cancel")).clicked() {
                    close = true;
                }
            });
        });
    });
    if submit && !d.busy {
        let email = d.email.trim().to_owned();
        if !email.contains('@') || email.len() > 320 {
            d.error = Some(t("Type the email address of the account.").to_owned());
        } else if d.password.chars().count() < 6 {
            d.error = Some(t("The password needs at least 6 characters.").to_owned());
        } else {
            d.busy = true;
            d.error = None;
            d.info = None;
            spawn_sign_in(email, d.password.clone(), d.mode);
        }
    }
    if close || (r.should_close() && !d.busy) {
        with(|c| c.save_after_sign_in = false);
        return;
    }
    with(|c| c.account = Some(d));
}

fn projects_window(app: &mut BimApp, ctx: &Context) {
    let Some(mut d) = with(|c| c.projects.take()) else { return };
    let current = with(|c| c.current.as_ref().map(|(id, _, _)| id.clone()));
    let saving = with(|c| c.saving.is_some());
    let (mut close, mut reload, mut save) = (false, false, false);
    let mut open: Option<Row> = None;
    let mut remove: Option<Row> = None;
    let dirty = app.dirty();
    let r = crate::popup::Modal::new(Id::new("cloud-projects"), t("My Projects")).width(560.0).show(ctx, |ui| {
        crate::popup::body(ui, |ui| {
            if let Some(email) = signed_in_email() {
                ui.label(RichText::new(format!("{} {email}", t("Signed in as"))).color(pal().muted));
            }
            ui.add_space(6.0);
            if let Some(e) = &d.error {
                ui.colored_label(pal().error, e);
                ui.add_space(6.0);
            }
            match &d.rows {
                None => {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(t("Loading your projects…"));
                    });
                }
                Some(rows) if rows.is_empty() => {
                    ui.label(t("No projects in the cloud yet. Save to Cloud keeps the project on screen here."));
                }
                Some(rows) => {
                    if dirty {
                        ui.label(
                            RichText::new(t("Opening a project replaces the one on screen, and its unsaved changes.")).color(pal().muted),
                        );
                        ui.add_space(4.0);
                    }
                    ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                        egui::Grid::new("cloud-projects-grid").num_columns(4).striped(true).spacing([14.0, 6.0]).show(ui, |ui| {
                            ui.label(RichText::new(t("Name")).strong());
                            ui.label(RichText::new(t("Saved")).strong());
                            ui.label(RichText::new(t("Size")).strong());
                            ui.label("");
                            ui.end_row();
                            for row in rows {
                                let mut name = RichText::new(&row.name);
                                if current.as_deref() == Some(row.id.as_str()) {
                                    name = name.strong().color(pal().accent);
                                }
                                ui.label(name);
                                ui.label(when_text(&row.updated_at));
                                ui.label(size_text(row.size));
                                ui.horizontal(|ui| {
                                    ui.add_enabled_ui(!d.busy, |ui| {
                                        if d.confirm.as_deref() == Some(row.id.as_str()) {
                                            ui.colored_label(pal().error, t("Delete?"));
                                            if ui.button(t("Yes")).clicked() {
                                                remove = Some(row.clone());
                                            }
                                            if ui.button(t("No")).clicked() {
                                                d.confirm = None;
                                            }
                                        } else {
                                            if ui.button(t("Open")).clicked() {
                                                open = Some(row.clone());
                                            }
                                            if ui.button(t("Delete")).clicked() {
                                                d.confirm = Some(row.id.clone());
                                            }
                                        }
                                    });
                                });
                                ui.end_row();
                            }
                        });
                    });
                }
            }
            if d.busy && d.rows.is_some() {
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(t("Talking to the cloud…"));
                });
            }
        });
        crate::popup::footer(ui, |ui| {
            ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                if ui.button(t("Close")).clicked() {
                    close = true;
                }
                if ui.add_enabled(!saving, egui::Button::new(t("Save Current to Cloud"))).clicked() {
                    save = true;
                }
                if ui.add_enabled(!d.busy, egui::Button::new(t("Refresh"))).clicked() {
                    reload = true;
                }
            });
        });
    });
    if close || r.should_close() {
        return;
    }
    if let Some(row) = open {
        d.busy = true;
        d.error = None;
        wasm_bindgen_futures::spawn_local(async move { push(Event::Opened(download(row).await)) });
    }
    if let Some(row) = remove {
        d.busy = true;
        d.error = None;
        d.confirm = None;
        wasm_bindgen_futures::spawn_local(async move { push(Event::Deleted(delete(row).await)) });
    }
    with(|c| {
        if c.projects.is_none() {
            c.projects = Some(d);
        }
    });
    if reload {
        spawn_list();
    }
    if save {
        save_to_cloud(app);
    }
}

/// The account's chip on the app bar: "Sign in", or the email with a menu of the cloud's commands.
pub(crate) fn account_chip(app: &mut BimApp, ui: &mut Ui, at: egui::Rect) -> egui::Rect {
    let email = signed_in_email();
    let label = match &email {
        Some(e) => {
            let short: String = e.chars().take(22).collect();
            if short.len() < e.len() { format!("{short}…") } else { short }
        }
        None => t("Sign in").to_owned(),
    };
    let galley = ui.painter().layout_no_wrap(label.clone(), egui::FontId::proportional(12.5), pal().app_bar_text);
    let width = galley.size().x + 20.0;
    let rect = egui::Rect::from_min_size(egui::pos2(at.left() - width - 8.0, at.top()), egui::vec2(width, at.height()));
    let button = egui::Button::new(RichText::new(&label).color(pal().app_bar_text).size(12.5))
        .fill(if email.is_some() { egui::Color32::TRANSPARENT } else { pal().accent.gamma_multiply(0.55) })
        .corner_radius(12.0);
    let r = ui.put(rect, button);
    if email.is_none() {
        if r.clicked() {
            open_account(Mode::SignIn);
        }
        return rect;
    }
    let r = r.on_hover_text(t("Your CampusBIM account"));
    egui::Popup::menu(&r).show(|ui| {
        if let Some(e) = &email {
            ui.label(RichText::new(e).color(pal().muted));
            ui.separator();
        }
        cloud_items(app, ui);
    });
    rect
}

/// File menu items: Save to Cloud, My Projects, and signing in or out.
pub(crate) fn cloud_items(app: &mut BimApp, ui: &mut Ui) {
    if ui.button(t("Save to Cloud")).clicked() {
        save_to_cloud(app);
        ui.close();
    }
    if ui.button(t("My Projects…")).clicked() {
        open_projects();
        ui.close();
    }
    if signed_in_email().is_some() {
        if ui.button(t("Sign Out")).clicked() {
            sign_out(app);
            ui.close();
        }
    } else if ui.button(t("Sign In…")).clicked() {
        open_account(Mode::SignIn);
        ui.close();
    }
}
