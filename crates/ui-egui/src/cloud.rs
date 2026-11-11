//! CampusBIM accounts and cloud save, in the browser only: sign up, sign in and sign out with an email and a password,
//! Save to Cloud (the project and a picture of the view), the start page's dashboard of the user's projects (open, rename,
//! duplicate, delete), and Forgot password / Set new password / Change password. It talks to Supabase's Auth, REST and Storage HTTP APIs with the page's own `fetch`;
//! the session (its access and refresh tokens) is kept in `localStorage` and refreshed before it runs out.
//!
//! The project URL and the publishable key are public (Row Level Security keeps each user to their own rows and files),
//! so they are built in: `CAMPUSBIM_SUPABASE_URL` and `CAMPUSBIM_SUPABASE_KEY` at build time replace the defaults.

use std::cell::RefCell;
use std::path::PathBuf;
use std::sync::Arc;

use bimcraft_core::model::Model;
use egui::{Align, Context, Id, Key, Layout, RichText, Sense, TextEdit, Ui, vec2};
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
    /// Forgot password: the email to send Supabase's recovery link to.
    Reset,
}

struct AccountDialog {
    email: String,
    password: String,
    mode: Mode,
    busy: bool,
    error: Option<String>,
    info: Option<String>,
}

/// Set new password (from a recovery link) or Change password (from the account menu).
struct PasswordDialog {
    recovery: bool,
    password: String,
    again: String,
    busy: bool,
    error: Option<String>,
}

#[derive(Clone)]
struct Row {
    id: String,
    name: String,
    updated_at: String,
    size: i64,
    path: String,
    thumb: Option<String>,
}

enum Thumb {
    Loading,
    Ready(egui::TextureHandle),
    None,
}

/// What the dashboard asks the start page to do.
pub(crate) enum DashClick {
    New,
    Open,
}

enum Event {
    /// `None`: the account was made but has to be confirmed by email first.
    SignedIn(Result<Option<Auth>, String>),
    Refreshed(Result<Auth, String>),
    Saved(Result<(String, String, usize), String>),
    Listed(Result<Vec<Row>, String>),
    Opened(Result<(String, String, Vec<u8>), String>),
    Deleted(Result<String, String>),
    Renamed(Result<(String, String), String>),
    Duplicated(Result<String, String>),
    ThumbLoaded(String, Option<Vec<u8>>),
    ResetSent(Result<(), String>),
    /// A recovery link's session, with the user it belongs to.
    Recovered(Result<Auth, String>),
    PasswordSet(Result<(), String>),
}

#[derive(Default)]
struct Cloud {
    auth: Option<Auth>,
    loaded: bool,
    account: Option<AccountDialog>,
    password: Option<PasswordDialog>,
    /// The signed-in user's projects, newest first; none until listed.
    rows: Option<Vec<Row>>,
    listing: bool,
    list_error: Option<String>,
    /// What a card's menu is waiting on (rename, duplicate, delete, open), so the cards hold still.
    working: Option<String>,
    thumbs: std::collections::HashMap<String, (String, Thumb)>,
    search: String,
    renaming: Option<(String, String)>,
    deleting: Option<Row>,
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
        Mode::SignIn | Mode::Reset => format!("{}/auth/v1/token?grant_type=password", base_url()),
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

fn object_url(path: &str) -> String {
    format!("{}/storage/v1/object/{BUCKET}/{}", base_url(), encode_path(path))
}

async fn put_object(bearer: &str, path: &str, bytes: Vec<u8>, mime: &str) -> Result<(), String> {
    json_call(
        "POST",
        &object_url(path),
        &[("Authorization", bearer.to_owned()), ("x-upsert", "true".into()), ("Content-Type", mime.into())],
        Some(Body::Bytes(bytes)),
    )
    .await
    .map(|_| ())
}

async fn upload(id: String, name: String, bytes: Vec<u8>, thumb: Option<Vec<u8>>) -> Result<(String, String, usize), String> {
    let (bearer, user) = bearer().await?;
    let path = format!("{user}/{id}.bimcraft");
    let size = bytes.len();
    put_object(&bearer, &path, bytes, "application/json").await?;
    let mut row = json!({"id": id, "name": name, "size_bytes": size, "storage_path": path});
    // A picture that doesn't go up leaves the project saved, with the card's placeholder (or its last picture).
    if let Some(png) = thumb {
        let thumb_path = format!("{user}/{id}.png");
        match put_object(&bearer, &thumb_path, png, "image/png").await {
            Ok(()) => row["thumb_path"] = json!(thumb_path),
            Err(e) => web_sys::console::warn_1(&format!("bimcraft: the project's picture wasn't saved: {e}").into()),
        }
    }
    let url = format!("{}/rest/v1/projects?on_conflict=id", base_url());
    json_call(
        "POST",
        &url,
        &[("Authorization", bearer), ("Prefer", "resolution=merge-duplicates,return=minimal".into())],
        Some(Body::Json(row)),
    )
    .await?;
    Ok((id, name, size))
}

async fn list() -> Result<Vec<Row>, String> {
    let (bearer, _) = bearer().await?;
    let url = format!("{}/rest/v1/projects?select=id,name,updated_at,size_bytes,storage_path,thumb_path&order=updated_at.desc", base_url());
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
                thumb: r.get("thumb_path").and_then(Value::as_str).map(str::to_owned),
            })
        })
        .collect())
}

async fn get_object(path: &str) -> Result<Vec<u8>, String> {
    let (bearer, _) = bearer().await?;
    let url = format!("{}/storage/v1/object/authenticated/{BUCKET}/{}", base_url(), encode_path(path));
    let (status, bytes) = http("GET", &url, &[("Authorization", bearer)], None).await?;
    if !(200..300).contains(&status) {
        return Err(error_text(status, &bytes));
    }
    Ok(bytes)
}

/// The file, with its project's name made the card's (a rename or a copy changes only the row).
async fn download(row: Row) -> Result<(String, String, Vec<u8>), String> {
    let bytes = get_object(&row.path).await?;
    let bytes = match serde_json::from_slice::<Value>(&bytes) {
        Ok(mut v) if v.get("name").is_some_and(Value::is_string) && v["name"].as_str() != Some(row.name.as_str()) => {
            v["name"] = json!(row.name);
            serde_json::to_vec(&v).unwrap_or(bytes)
        }
        _ => bytes,
    };
    Ok((row.id, row.name, bytes))
}

async fn delete(row: Row) -> Result<String, String> {
    let (bearer, _) = bearer().await?;
    let mut prefixes = vec![row.path.clone()];
    prefixes.extend(row.thumb.clone());
    let url = format!("{}/storage/v1/object/{BUCKET}", base_url());
    let (status, bytes) =
        http("DELETE", &url, &[("Authorization", bearer.clone())], Some(Body::Json(json!({"prefixes": prefixes})))).await?;
    // A row whose file is already gone can still be deleted.
    if !(200..300).contains(&status) && status != 404 {
        return Err(error_text(status, &bytes));
    }
    let url = format!("{}/rest/v1/projects?id=eq.{}", base_url(), js_sys::encode_uri_component(&row.id));
    json_call("DELETE", &url, &[("Authorization", bearer)], None).await?;
    Ok(row.id)
}

async fn rename(id: String, name: String) -> Result<(String, String), String> {
    let (bearer, _) = bearer().await?;
    let url = format!("{}/rest/v1/projects?id=eq.{}", base_url(), js_sys::encode_uri_component(&id));
    json_call("PATCH", &url, &[("Authorization", bearer), ("Prefer", "return=minimal".into())], Some(Body::Json(json!({"name": name}))))
        .await?;
    Ok((id, name))
}

async fn copy_object(bearer: &str, from: &str, to: &str) -> Result<(), String> {
    let url = format!("{}/storage/v1/object/copy", base_url());
    json_call(
        "POST",
        &url,
        &[("Authorization", bearer.to_owned())],
        Some(Body::Json(json!({"bucketId": BUCKET, "sourceKey": from, "destinationKey": to}))),
    )
    .await
    .map(|_| ())
}

async fn duplicate(row: Row, id: String) -> Result<String, String> {
    let (bearer, user) = bearer().await?;
    let path = format!("{user}/{id}.bimcraft");
    copy_object(&bearer, &row.path, &path).await?;
    let mut thumb = None;
    if let Some(from) = &row.thumb {
        let to = format!("{user}/{id}.png");
        if copy_object(&bearer, from, &to).await.is_ok() {
            thumb = Some(to);
        }
    }
    let base: String = row.name.chars().take(190).collect();
    let name = format!("{base} {}", t("(copy)"));
    let url = format!("{}/rest/v1/projects", base_url());
    let body = json!({"id": id, "name": name, "size_bytes": row.size, "storage_path": path, "thumb_path": thumb});
    json_call("POST", &url, &[("Authorization", bearer), ("Prefer", "return=minimal".into())], Some(Body::Json(body))).await?;
    Ok(name)
}

/// The page's address without its hash: where a recovery link brings the user back to.
fn app_url() -> String {
    let Some(loc) = web_sys::window().map(|w| w.location()) else { return String::new() };
    format!("{}{}", loc.origin().unwrap_or_default(), loc.pathname().unwrap_or_default())
}

async fn send_reset(email: String) -> Result<(), String> {
    let url = format!("{}/auth/v1/recover?redirect_to={}", base_url(), String::from(js_sys::encode_uri_component(&app_url())));
    json_call("POST", &url, &[], Some(Body::Json(json!({"email": email})))).await.map(|_| ())
}

/// A recovery link's tokens, completed with the user they belong to.
async fn recovered(access: String, refresh: String, expires_at: f64) -> Result<Auth, String> {
    let url = format!("{}/auth/v1/user", base_url());
    let user = json_call("GET", &url, &[("Authorization", format!("Bearer {access}"))], None).await?;
    let id = user.get("id").and_then(Value::as_str).ok_or_else(|| t("The reset link didn't name an account.").to_owned())?;
    Ok(Auth { access, refresh, expires_at, user: id.to_owned(), email: user.get("email").and_then(Value::as_str).unwrap_or("").to_owned() })
}

async fn set_password(password: String) -> Result<(), String> {
    let (bearer, _) = bearer().await?;
    let url = format!("{}/auth/v1/user", base_url());
    json_call("PUT", &url, &[("Authorization", bearer)], Some(Body::Json(json!({"password": password})))).await.map(|_| ())
}

/// What a link from Supabase's email left in the address's hash: a recovery session, an error, or nothing for us.
enum Landing {
    Recovery { access: String, refresh: String, expires_at: f64 },
    Error(String),
}

fn read_hash(hash: &str) -> Option<Landing> {
    let hash = hash.strip_prefix('#').unwrap_or(hash);
    if hash.is_empty() {
        return None;
    }
    let mut fields = std::collections::HashMap::new();
    for pair in hash.split('&') {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        let v = js_sys::decode_uri_component(&v.replace('+', " ")).ok().map(String::from).unwrap_or_else(|| v.to_owned());
        fields.insert(k.to_owned(), v);
    }
    if let Some(e) = fields.get("error_description").or_else(|| fields.get("error")) {
        let expired = fields.get("error_code").is_some_and(|c| c == "otp_expired");
        return Some(Landing::Error(if expired {
            t("That reset link has expired or was already used. Ask for a new one with Forgot password.").to_owned()
        } else {
            e.clone()
        }));
    }
    if fields.get("type").map(String::as_str) != Some("recovery") {
        return None;
    }
    let access = fields.get("access_token")?.clone();
    let refresh = fields.get("refresh_token")?.clone();
    let expires_at = fields
        .get("expires_at")
        .and_then(|s| s.parse().ok())
        .or_else(|| fields.get("expires_in").and_then(|s| s.parse::<f64>().ok()).map(|s| now_secs() + s))
        .unwrap_or_else(|| now_secs() + 3600.0);
    Some(Landing::Recovery { access, refresh, expires_at })
}

/// Take the tokens out of the address, so a reload or a shared link doesn't carry them.
fn clear_hash() {
    let Some(window) = web_sys::window() else { return };
    let loc = window.location();
    let clean = format!("{}{}", loc.pathname().unwrap_or_default(), loc.search().unwrap_or_default());
    if let Ok(history) = window.history() {
        let _ = history.replace_state_with_url(&JsValue::NULL, "", Some(&clean));
    }
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

fn spawn<F: std::future::Future<Output = Event> + 'static>(f: F) {
    wasm_bindgen_futures::spawn_local(async move { push(f.await) });
}

/// List the user's projects again (the dashboard and My Projects show them).
fn refresh_list() {
    if with(|c| std::mem::replace(&mut c.listing, true)) {
        return;
    }
    with(|c| c.list_error = None);
    spawn(async { Event::Listed(list().await) });
}

fn set_message(app: &mut BimApp, text: String, error: bool) {
    with(|c| c.message.clone_from(&text));
    app.status = Status { text, error };
}

/// File > Save to Cloud: the project as a `.bimcraft` and a picture of the view, uploaded under the user's folder, its row
/// added or brought up to date.
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
    let thumb = app.view_png(THUMB_W, THUMB_H);
    let uid = app.session.model.uid;
    // A project from the cloud keeps its card's name (a rename changes only the card).
    let (id, name) = with(|c| c.current.as_ref().filter(|(_, _, u)| *u == uid).map(|(id, name, _)| (id.clone(), name.clone())))
        .unwrap_or_else(|| {
            let name = app.session.model.name.trim().to_owned();
            (new_id(), if name.is_empty() { "Untitled".to_owned() } else { name.chars().take(200).collect() })
        });
    with(|c| c.saving = Some(Arc::clone(&app.session.model)));
    set_message(app, format!("{} {name}…", t("Saving to the cloud:")), false);
    spawn(async move { Event::Saved(upload(id, name, bytes, thumb).await) });
}

const THUMB_W: u32 = 480;
const THUMB_H: u32 = 300;

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

/// My Projects: the start page's dashboard, over the project on screen.
pub(crate) fn open_projects(app: &mut BimApp) {
    if with(|c| c.auth.is_none()) {
        open_account(Mode::SignIn);
        return;
    }
    show_dashboard(app);
}

fn show_dashboard(app: &mut BimApp) {
    if !app.home.shown {
        (app.home.shown, app.home.over) = (true, true);
    }
    refresh_list();
}

fn open_password(recovery: bool) {
    with(|c| c.password = Some(PasswordDialog { recovery, password: String::new(), again: String::new(), busy: false, error: None }));
}

pub(crate) fn sign_out(app: &mut BimApp) {
    let auth = with(|c| {
        c.rows = None;
        c.thumbs.clear();
        c.current = None;
        c.renaming = None;
        c.deleting = None;
        c.password = None;
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

/// Load the session once (or take a recovery link's), refresh it before it runs out, apply what came back from the network,
/// and draw the dialogs.
pub(crate) fn poll(app: &mut BimApp, ctx: &Context) {
    REPAINT.with(|r| *r.borrow_mut() = Some(ctx.clone()));
    if !with(|c| std::mem::replace(&mut c.loaded, true)) {
        first_frame(app);
    }
    if let Some(token) = with(|c| match &c.auth {
        Some(a) if !c.refreshing && a.expires_at - now_secs() < 300.0 => {
            c.refreshing = true;
            Some(a.refresh.clone())
        }
        _ => None,
    }) {
        spawn(async move { Event::Refreshed(refresh(token).await) });
    }
    if let Some(a) = with(|c| c.auth.as_ref().map(|a| a.expires_at)) {
        let left = (a - now_secs() - 290.0).clamp(1.0, 3600.0);
        ctx.request_repaint_after(std::time::Duration::from_secs_f64(left));
    }
    for event in with(|c| std::mem::take(&mut c.events)) {
        apply(app, ctx, event);
    }
    let uid = app.session.model.uid;
    with(|c| {
        if c.current.as_ref().is_some_and(|(_, _, u)| *u != uid) {
            c.current = None;
        }
    });
    account_window(ctx);
    password_window(ctx);
    rename_window(ctx);
    delete_window(ctx);
    publish();
}

fn first_frame(app: &mut BimApp) {
    let hash = web_sys::window().and_then(|w| w.location().hash().ok()).unwrap_or_default();
    match read_hash(&hash) {
        Some(Landing::Recovery { access, refresh, expires_at }) => {
            clear_hash();
            open_password(true);
            with(|c| {
                if let Some(p) = c.password.as_mut() {
                    p.busy = true;
                }
            });
            spawn(async move { Event::Recovered(recovered(access, refresh, expires_at).await) });
            return;
        }
        Some(Landing::Error(e)) => {
            clear_hash();
            open_account(Mode::Reset);
            with(|c| {
                if let Some(a) = c.account.as_mut() {
                    a.error = Some(e);
                }
            });
        }
        None => {}
    }
    let auth = load_auth();
    let signed_in = auth.is_some();
    with(|c| c.auth = auth);
    if signed_in {
        show_dashboard(app);
    }
}

fn apply(app: &mut BimApp, ctx: &Context, event: Event) {
    match event {
        Event::SignedIn(Ok(Some(auth))) => {
            store_auth(Some(&auth));
            let email = auth.email.clone();
            let then_save = with(|c| {
                c.auth = Some(auth);
                c.account = None;
                c.rows = None;
                c.thumbs.clear();
                std::mem::take(&mut c.save_after_sign_in)
            });
            set_message(app, format!("{} {email}", t("Signed in as")), false);
            if then_save {
                save_to_cloud(app);
            } else {
                show_dashboard(app);
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
        Event::SignedIn(Err(e)) | Event::ResetSent(Err(e)) => with(|c| {
            if let Some(a) = c.account.as_mut() {
                a.busy = false;
                a.error = Some(e);
            }
        }),
        Event::ResetSent(Ok(())) => with(|c| {
            if let Some(a) = c.account.as_mut() {
                a.busy = false;
                a.info = Some(t("If there's an account for that email, a link to set a new password is on its way.").to_owned());
            }
        }),
        Event::Recovered(Ok(auth)) => {
            store_auth(Some(&auth));
            with(|c| {
                c.auth = Some(auth);
                if let Some(p) = c.password.as_mut() {
                    p.busy = false;
                }
            });
        }
        Event::Recovered(Err(e)) => {
            with(|c| c.password = None);
            open_account(Mode::Reset);
            with(|c| {
                if let Some(a) = c.account.as_mut() {
                    a.error = Some(format!("{} {e}", t("That reset link didn't work.")));
                }
            });
        }
        Event::PasswordSet(Ok(())) => {
            let recovery = with(|c| c.password.take().is_some_and(|p| p.recovery));
            let email = signed_in_email().unwrap_or_default();
            set_message(app, format!("{} {email}", t("Password changed. Signed in as")), false);
            if recovery {
                show_dashboard(app);
            }
        }
        Event::PasswordSet(Err(e)) => with(|c| {
            if let Some(p) = c.password.as_mut() {
                p.busy = false;
                p.error = Some(e);
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
            refresh_list();
        }
        Event::Saved(Err(e)) => {
            with(|c| c.saving = None);
            set_message(app, format!("{} {e}", t("Couldn't save to the cloud.")), true);
        }
        Event::Listed(result) => with(|c| {
            c.listing = false;
            match result {
                Ok(rows) => {
                    // Pictures of projects gone, or saved again since, are let go.
                    c.thumbs.retain(|id, (stamp, _)| rows.iter().any(|r| &r.id == id && &r.updated_at == stamp));
                    c.rows = Some(rows);
                }
                Err(e) => c.list_error = Some(e),
            }
        }),
        Event::ThumbLoaded(id, bytes) => {
            let image = bytes
                .and_then(|b| tiny_skia::Pixmap::decode_png(&b).ok())
                .map(|pm| egui::ColorImage::from_rgba_premultiplied([pm.width() as usize, pm.height() as usize], pm.data()));
            let thumb = match image {
                Some(image) => Thumb::Ready(ctx.load_texture(format!("cloud-thumb-{id}"), image, egui::TextureOptions::LINEAR)),
                None => Thumb::None,
            };
            with(|c| {
                if let Some((_, t)) = c.thumbs.get_mut(&id) {
                    *t = thumb;
                }
            });
        }
        Event::Opened(Ok((id, name, bytes))) => {
            with(|c| c.working = None);
            let file = format!("{}.bimcraft", name.replace(['/', '\\'], "-"));
            let path = PathBuf::from(&file);
            app.session.provide_bytes(&path, bytes);
            if app.open_path(&path) {
                let uid = app.session.model.uid;
                with(|c| c.current = Some((id, name.clone(), uid)));
                app.home.shown = false;
                set_message(app, format!("{} {name}", t("Opened from the cloud:")), false);
            } else {
                let e = app.status.text.clone();
                with(|c| c.list_error = Some(e));
            }
        }
        Event::Opened(Err(e)) | Event::Deleted(Err(e)) | Event::Renamed(Err(e)) | Event::Duplicated(Err(e)) => {
            with(|c| {
                c.working = None;
                c.list_error = Some(e.clone());
            });
            set_message(app, e, true);
        }
        Event::Deleted(Ok(id)) => {
            with(|c| {
                c.working = None;
                if c.current.as_ref().is_some_and(|(cur, _, _)| *cur == id) {
                    c.current = None;
                }
                if let Some(rows) = c.rows.as_mut() {
                    rows.retain(|r| r.id != id);
                }
                c.thumbs.remove(&id);
            });
            set_message(app, t("Deleted from the cloud.").to_owned(), false);
        }
        Event::Renamed(Ok((id, name))) => {
            with(|c| {
                c.working = None;
                if let Some(cur) = c.current.as_mut().filter(|(cur, _, _)| *cur == id) {
                    cur.1.clone_from(&name);
                }
                if let Some(row) = c.rows.as_mut().and_then(|rows| rows.iter_mut().find(|r| r.id == id)) {
                    row.name.clone_from(&name);
                }
            });
            set_message(app, format!("{} {name}", t("Renamed to")), false);
        }
        Event::Duplicated(Ok(name)) => {
            with(|c| c.working = None);
            set_message(app, format!("{} {name}", t("Duplicated as")), false);
            refresh_list();
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
            "working": c.working,
            "accountOpen": c.account.is_some(),
            "accountMode": c.account.as_ref().map(|a| match a.mode { Mode::SignIn => "signIn", Mode::SignUp => "signUp", Mode::Reset => "reset" }),
            "accountError": c.account.as_ref().and_then(|a| a.error.clone()),
            "accountInfo": c.account.as_ref().and_then(|a| a.info.clone()),
            "passwordOpen": c.password.as_ref().map(|p| if p.recovery { "recovery" } else { "change" }),
            "passwordError": c.password.as_ref().and_then(|p| p.error.clone()),
            "projects": c.rows.as_ref().map(|rows| rows.iter().map(|r| json!({
                "id": r.id, "name": r.name, "size": r.size, "thumb": r.thumb.is_some(),
                "thumbShown": matches!(c.thumbs.get(&r.id), Some((_, Thumb::Ready(_)))),
            })).collect::<Vec<_>>()),
            "projectsError": c.list_error,
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

fn primary(ui: &mut Ui, enabled: bool, label: &str) -> bool {
    let b = egui::Button::new(RichText::new(label).strong().color(egui::Color32::WHITE)).fill(pal().accent);
    ui.add_enabled(enabled, b).clicked()
}

fn busy_line(ui: &mut Ui) {
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.spinner();
        ui.label(t("Talking to the cloud…"));
    });
}

fn account_window(ctx: &Context) {
    let Some(mut d) = with(|c| c.account.take()) else { return };
    let mut close = false;
    let mut submit = false;
    let r = crate::popup::Modal::new(Id::new("cloud-account"), t("CampusBIM account")).width(400.0).show(ctx, |ui| {
        crate::popup::body(ui, |ui| {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.add_enabled_ui(!d.busy, |ui| {
                    if ui.selectable_label(d.mode != Mode::SignUp, t("Sign in")).clicked() {
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
                    Mode::Reset => "Type your account's email and we'll send a link to set a new password.",
                }))
                .color(pal().muted),
            );
            ui.add_space(8.0);
            ui.label(t("Email"));
            let email =
                ui.add_enabled(!d.busy, TextEdit::singleline(&mut d.email).hint_text("you@school.edu.ng").desired_width(f32::INFINITY));
            let mut enter = email.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
            // Typing goes to a field, never to the window's shortcuts behind the dialog.
            let unfocused = !d.busy && ui.memory(|m| m.focused().is_none());
            if d.mode != Mode::Reset {
                ui.add_space(4.0);
                ui.label(t("Password"));
                let pass = ui.add_enabled(!d.busy, TextEdit::singleline(&mut d.password).password(true).desired_width(f32::INFINITY));
                enter |= pass.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
                if unfocused && !d.email.trim().is_empty() && !enter {
                    pass.request_focus();
                }
            }
            if unfocused && (d.email.trim().is_empty() || d.mode == Mode::Reset) && !enter {
                email.request_focus();
            }
            if d.mode == Mode::SignIn {
                ui.add_space(2.0);
                if ui.add_enabled(!d.busy, egui::Link::new(t("Forgot password?"))).clicked() {
                    d.mode = Mode::Reset;
                    d.error = None;
                    d.info = None;
                }
            } else if d.mode == Mode::Reset {
                ui.add_space(2.0);
                if ui.add_enabled(!d.busy, egui::Link::new(t("Back to sign in"))).clicked() {
                    d.mode = Mode::SignIn;
                    d.error = None;
                    d.info = None;
                }
            }
            submit |= enter;
            if let Some(e) = &d.error {
                ui.add_space(6.0);
                ui.colored_label(pal().error, e);
            }
            if let Some(i) = &d.info {
                ui.add_space(6.0);
                ui.colored_label(pal().accent, i);
            }
            if d.busy {
                busy_line(ui);
            }
        });
        crate::popup::footer(ui, |ui| {
            ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                let label = t(match d.mode {
                    Mode::SignIn => "Sign in",
                    Mode::SignUp => "Create account",
                    Mode::Reset => "Send reset link",
                });
                submit |= primary(ui, !d.busy, label);
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
        } else if d.mode != Mode::Reset && d.password.chars().count() < 6 {
            d.error = Some(t("The password needs at least 6 characters.").to_owned());
        } else {
            d.busy = true;
            d.error = None;
            d.info = None;
            let mode = d.mode;
            let password = d.password.clone();
            match mode {
                Mode::Reset => spawn(async move { Event::ResetSent(send_reset(email).await) }),
                _ => spawn(async move { Event::SignedIn(sign_in(email, password, mode).await) }),
            }
        }
    }
    if close || (r.should_close() && !d.busy) {
        with(|c| c.save_after_sign_in = false);
        return;
    }
    with(|c| c.account = Some(d));
}

fn password_window(ctx: &Context) {
    let Some(mut d) = with(|c| c.password.take()) else { return };
    let (mut close, mut submit) = (false, false);
    let title = t(if d.recovery { "Set new password" } else { "Change password" });
    let r = crate::popup::Modal::new(Id::new("cloud-password"), title).width(400.0).closable(!d.recovery || !d.busy).show(ctx, |ui| {
        crate::popup::body(ui, |ui| {
            ui.add_space(4.0);
            if let Some(email) = signed_in_email() {
                ui.label(RichText::new(email).color(pal().muted));
                ui.add_space(6.0);
            }
            ui.label(t("New password"));
            let a = ui.add_enabled(!d.busy, TextEdit::singleline(&mut d.password).password(true).desired_width(f32::INFINITY));
            ui.add_space(4.0);
            ui.label(t("Type it again"));
            let b = ui.add_enabled(!d.busy, TextEdit::singleline(&mut d.again).password(true).desired_width(f32::INFINITY));
            submit |= (a.lost_focus() || b.lost_focus()) && ui.input(|i| i.key_pressed(Key::Enter));
            if !d.busy && !submit && ui.memory(|m| m.focused().is_none()) {
                if d.password.is_empty() { a.request_focus() } else { b.request_focus() }
            }
            if let Some(e) = &d.error {
                ui.add_space(6.0);
                ui.colored_label(pal().error, e);
            }
            if d.busy {
                busy_line(ui);
            }
        });
        crate::popup::footer(ui, |ui| {
            ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                submit |= primary(ui, !d.busy, t("Save password"));
                if ui.button(t("Cancel")).clicked() {
                    close = true;
                }
            });
        });
    });
    if submit && !d.busy {
        if d.password.chars().count() < 6 {
            d.error = Some(t("The password needs at least 6 characters.").to_owned());
        } else if d.password != d.again {
            d.error = Some(t("The two passwords are different.").to_owned());
        } else if with(|c| c.auth.is_none()) {
            d.error = Some(t("Sign in first.").to_owned());
        } else {
            d.busy = true;
            d.error = None;
            let password = d.password.clone();
            spawn(async move { Event::PasswordSet(set_password(password).await) });
        }
    }
    if close || (r.should_close() && !d.busy) {
        return;
    }
    with(|c| c.password = Some(d));
}

fn rename_window(ctx: &Context) {
    let Some((id, mut name)) = with(|c| c.renaming.take()) else { return };
    let (mut close, mut submit) = (false, false);
    let r = crate::popup::Modal::new(Id::new("cloud-rename"), t("Rename project")).width(380.0).show(ctx, |ui| {
        crate::popup::body(ui, |ui| {
            ui.add_space(4.0);
            ui.label(t("Name"));
            let field = ui.add(TextEdit::singleline(&mut name).desired_width(f32::INFINITY).char_limit(200));
            if !field.has_focus() && !field.lost_focus() && ui.memory(|m| m.focused().is_none()) {
                field.request_focus();
            }
            submit |= field.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
        });
        crate::popup::footer(ui, |ui| {
            ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                submit |= primary(ui, !name.trim().is_empty(), t("Rename"));
                if ui.button(t("Cancel")).clicked() {
                    close = true;
                }
            });
        });
    });
    if submit && !name.trim().is_empty() {
        let name = name.trim().to_owned();
        with(|c| c.working = Some(id.clone()));
        spawn(async move { Event::Renamed(rename(id, name).await) });
        return;
    }
    if close || r.should_close() {
        return;
    }
    with(|c| c.renaming = Some((id, name)));
}

fn delete_window(ctx: &Context) {
    let Some(row) = with(|c| c.deleting.take()) else { return };
    let (mut close, mut yes) = (false, false);
    let r = crate::popup::Modal::new(Id::new("cloud-delete"), t("Delete project?")).width(400.0).show(ctx, |ui| {
        crate::popup::body(ui, |ui| {
            ui.add_space(4.0);
            ui.label(RichText::new(&row.name).strong());
            ui.add_space(4.0);
            ui.label(t("This deletes the project and its picture from the cloud. It can't be undone."));
        });
        crate::popup::footer(ui, |ui| {
            ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                let b = egui::Button::new(RichText::new(t("Delete")).strong().color(egui::Color32::WHITE)).fill(pal().error);
                yes = ui.add(b).clicked();
                if ui.button(t("Cancel")).clicked() {
                    close = true;
                }
            });
        });
    });
    if yes {
        with(|c| c.working = Some(row.id.clone()));
        spawn(async move { Event::Deleted(delete(row).await) });
        return;
    }
    if close || r.should_close() {
        return;
    }
    with(|c| c.deleting = Some(row));
}

// ---------- the dashboard ----------

const CARD: f32 = 232.0;
const CARD_PIC: f32 = 145.0;
const CARD_GAP: f32 = 18.0;

/// The start page's right side in the browser: the signed-in user's cloud projects as cards, or a prompt to sign in.
pub(crate) fn dashboard(app: &BimApp, ui: &mut Ui) -> Option<DashClick> {
    let Some(email) = signed_in_email() else {
        signed_out_prompt(ui);
        return None;
    };
    let _ = app;
    let mut click = None;
    crate::start::heading(ui, "Your cloud projects");
    ui.horizontal(|ui| {
        let mut search = with(|c| c.search.clone());
        let field = TextEdit::singleline(&mut search).hint_text(t("Search projects")).desired_width(220.0);
        if ui.add(field).changed() {
            with(|c| c.search.clone_from(&search));
        }
        if ui.button(t("New project")).clicked() {
            click = Some(DashClick::New);
        }
        if ui.button(t("Open file…")).clicked() {
            click = Some(DashClick::Open);
        }
        let listing = with(|c| c.listing);
        if ui.add_enabled(!listing, egui::Button::new(t("Refresh"))).clicked() {
            refresh_list();
        }
        if listing {
            ui.spinner();
        }
    });
    ui.horizontal(|ui| {
        ui.label(RichText::new(format!("{} {email}", t("Signed in as"))).size(12.0).color(pal().muted));
    });
    ui.add_space(12.0);
    if let Some(e) = with(|c| c.list_error.clone()) {
        ui.colored_label(pal().error, e);
        ui.add_space(8.0);
    }
    let (rows, search) = with(|c| (c.rows.clone(), c.search.trim().to_lowercase()));
    let Some(rows) = rows else {
        if !with(|c| c.listing) {
            refresh_list();
        }
        ui.horizontal(|ui| {
            ui.spinner();
            ui.label(t("Loading your projects…"));
        });
        return click;
    };
    let shown: Vec<&Row> = rows.iter().filter(|r| search.is_empty() || r.name.to_lowercase().contains(&search)).collect();
    if shown.is_empty() {
        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 150.0), Sense::hover());
        crate::start::dashed(ui, rect.shrink(1.0));
        let inner = egui::Rect::from_center_size(rect.center(), vec2(rect.width(), 48.0));
        ui.scope_builder(egui::UiBuilder::new().max_rect(inner).layout(Layout::top_down(Align::Center)), |ui| {
            if rows.is_empty() {
                ui.label(RichText::new(t("No cloud projects yet")).size(15.0).color(pal().text));
                ui.label(
                    RichText::new(t("Open or start a project, then File > Save to Cloud keeps it here.")).size(13.0).color(pal().muted),
                );
            } else {
                ui.label(RichText::new(t("No projects match the search")).size(15.0).color(pal().text));
            }
        });
        return click;
    }
    let working = with(|c| c.working.clone());
    let current = with(|c| c.current.as_ref().map(|(id, _, _)| id.clone()));
    let width = ui.available_width();
    let per_row = (((width + CARD_GAP) / (CARD + CARD_GAP)).floor() as usize).max(1);
    for chunk in shown.chunks(per_row) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = CARD_GAP;
            for row in chunk {
                card(ui, row, working.as_deref(), current.as_deref() == Some(row.id.as_str()));
            }
        });
        ui.add_space(CARD_GAP);
    }
    click
}

fn signed_out_prompt(ui: &mut Ui) {
    crate::start::heading(ui, "Cloud projects");
    let frame = egui::Frame::new()
        .fill(pal().surface)
        .stroke(egui::Stroke::new(1.0, pal().border))
        .corner_radius(egui::CornerRadius::same(12))
        .inner_margin(egui::Margin::same(18));
    frame.show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.label(RichText::new(t("Sign in to see your cloud projects")).size(16.0).strong().color(pal().text));
        ui.add_space(4.0);
        ui.label(RichText::new(t("Save projects to your free CampusBIM account and open them on any computer.")).color(pal().muted));
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            if primary(ui, true, t("Sign in")) {
                open_account(Mode::SignIn);
            }
            if ui.button(t("Create account")).clicked() {
                open_account(Mode::SignUp);
            }
        });
    });
}

/// One cloud project: its picture, name, when it was last saved and its size; a click opens it, its ⋯ menu renames,
/// duplicates and deletes it.
fn card(ui: &mut Ui, row: &Row, working: Option<&str>, current: bool) {
    use egui::{Align2, CornerRadius, FontId, Rect, Stroke, StrokeKind, epaint::Shadow, pos2};
    let busy = working.is_some();
    let (rect, r) = ui.allocate_exact_size(vec2(CARD, CARD_PIC + 62.0), if busy { Sense::hover() } else { Sense::click() });
    r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, !busy, &row.name));
    let hot = !busy && (r.hovered() || r.has_focus());
    let rect = if hot { rect.translate(vec2(0.0, -2.0)) } else { rect };
    let round = CornerRadius::same(12);
    {
        let p = ui.painter();
        let shadow =
            Shadow { offset: [0, if hot { 8 } else { 3 }], blur: if hot { 22 } else { 10 }, spread: 0, color: crate::start::shade(hot) };
        p.add(shadow.as_shape(rect, round));
        let edge = if hot || current { pal().accent } else { pal().border };
        p.rect(rect, round, pal().surface, Stroke::new(if hot || current { 1.5 } else { 1.0 }, edge), StrokeKind::Inside);
    }
    let pic = Rect::from_min_size(rect.min + vec2(1.0, 1.0), vec2(CARD - 2.0, CARD_PIC));
    let pic_round = CornerRadius { nw: 11, ne: 11, sw: 0, se: 0 };
    let thumb = thumb_for(row);
    match thumb {
        Some(tex) => {
            ui.painter().rect_filled(pic, pic_round, egui::Color32::WHITE);
            // Fill the card's picture, cropping the drawing's sides or top to keep its shape.
            let [w, h] = tex.size().map(|v| v as f32);
            let (tw, th) = (pic.width(), pic.height());
            let scale = (tw / w).max(th / h);
            let (cw, ch) = (tw / scale / w, th / scale / h);
            let uv = Rect::from_min_size(pos2((1.0 - cw) / 2.0, (1.0 - ch) / 2.0), vec2(cw, ch));
            egui::Image::new((tex.id(), pic.size())).uv(uv).corner_radius(pic_round).paint_at(ui, pic);
        }
        None => {
            let p = ui.painter();
            let tint = if crate::style::dark() { pal().bar } else { pal().hover_fill };
            p.rect_filled(pic, pic_round, tint);
            let icon = Rect::from_center_size(pic.center() - vec2(0.0, 6.0), vec2(40.0, 40.0));
            crate::icons::paint(p, crate::icons::Icon::Plan, icon, pal().muted);
            let note = if row.thumb.is_some() && with(|c| matches!(c.thumbs.get(&row.id), Some((_, Thumb::Loading)))) {
                t("Loading picture…")
            } else {
                t("No picture yet")
            };
            p.text(pic.center() + vec2(0.0, 30.0), Align2::CENTER_CENTER, note, FontId::proportional(12.0), pal().muted);
        }
    }
    let p = ui.painter();
    p.line_segment([pic.left_bottom(), pic.right_bottom()], Stroke::new(1.0, pal().border));
    let text_at = pos2(rect.left() + 14.0, pic.bottom() + 12.0);
    let clip = Rect::from_min_max(text_at, pos2(rect.right() - 40.0, rect.bottom()));
    let pc = p.with_clip_rect(clip);
    pc.text(text_at, Align2::LEFT_TOP, &row.name, FontId::proportional(14.5), pal().text);
    let detail = format!("{} · {}", when_text(&row.updated_at), size_text(row.size));
    pc.text(text_at + vec2(0.0, 22.0), Align2::LEFT_TOP, detail, FontId::proportional(12.0), pal().muted);
    if working == Some(row.id.as_str()) {
        let at = Rect::from_center_size(pic.center(), vec2(28.0, 28.0));
        ui.put(at, egui::Spinner::new().size(24.0));
    }
    // The ⋯ menu, at the bottom right.
    let dots = Rect::from_min_size(pos2(rect.right() - 36.0, pic.bottom() + 14.0), vec2(28.0, 28.0));
    let menu = ui.interact(dots, Id::new(("cloud-card-menu", &row.id)), if busy { Sense::hover() } else { Sense::click() });
    menu.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, !busy, t("Project menu")));
    if menu.hovered() {
        ui.painter().rect_filled(dots, 6.0, pal().hover_fill);
    }
    for dx in [-6.0, 0.0, 6.0] {
        ui.painter().circle_filled(dots.center() + vec2(dx, 0.0), 1.8, if menu.hovered() { pal().text } else { pal().muted });
    }
    let menu = menu.on_hover_text(t("Rename, duplicate or delete"));
    let mut open = !busy && r.clicked();
    let items = |ui: &mut Ui, open: &mut bool| {
        if ui.button(t("Open")).clicked() {
            *open = true;
        }
        if ui.button(t("Rename…")).clicked() {
            with(|c| c.renaming = Some((row.id.clone(), row.name.clone())));
        }
        if ui.button(t("Duplicate")).clicked() {
            let (copy, id) = (row.clone(), new_id());
            with(|c| c.working = Some(row.id.clone()));
            spawn(async move { Event::Duplicated(duplicate(copy, id).await) });
        }
        if ui.button(RichText::new(t("Delete…")).color(pal().error)).clicked() {
            with(|c| c.deleting = Some(row.clone()));
        }
    };
    if !busy {
        egui::Popup::menu(&menu).show(|ui| items(ui, &mut open));
        r.context_menu(|ui| items(ui, &mut open));
    }
    if open {
        let row = row.clone();
        with(|c| c.working = Some(row.id.clone()));
        spawn(async move { Event::Opened(download(row).await) });
    }
}

/// The card's picture when it's in; starts fetching it the first time.
fn thumb_for(row: &Row) -> Option<egui::TextureHandle> {
    let path = row.thumb.clone()?;
    let start = with(|c| match c.thumbs.get(&row.id) {
        Some((stamp, Thumb::Ready(tex))) if *stamp == row.updated_at => Err(Some(tex.clone())),
        Some((stamp, _)) if *stamp == row.updated_at => Err(None),
        _ => {
            c.thumbs.insert(row.id.clone(), (row.updated_at.clone(), Thumb::Loading));
            Ok(())
        }
    });
    match start {
        Err(tex) => tex,
        Ok(()) => {
            let id = row.id.clone();
            spawn(async move { Event::ThumbLoaded(id, get_object(&path).await.ok()) });
            None
        }
    }
}

/// The account's chip on the app bar: "Sign in", or the email with a menu of the cloud's commands. `at` is what's to its
/// right; the chip's own rect is returned.
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

/// File menu items: Save to Cloud, My Projects, and the account's.
pub(crate) fn cloud_items(app: &mut BimApp, ui: &mut Ui) {
    if ui.button(t("Save to Cloud")).clicked() {
        save_to_cloud(app);
        ui.close();
    }
    if ui.button(t("My Projects…")).clicked() {
        open_projects(app);
        ui.close();
    }
    if signed_in_email().is_some() {
        if ui.button(t("Change Password…")).clicked() {
            open_password(false);
            ui.close();
        }
        if ui.button(t("Sign Out")).clicked() {
            sign_out(app);
            ui.close();
        }
    } else if ui.button(t("Sign In…")).clicked() {
        open_account(Mode::SignIn);
        ui.close();
    }
}
