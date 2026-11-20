//! Playback through libmpv.
//!
//! mpv draws into the same OpenGL surface as egui so that the controls can
//! overlay the picture. Discovery order: MPV_DLL, the application directory,
//! its runtime subdirectory, then the Windows loader and PATH.

use std::ffi::{c_char, c_int, c_void, CStr, CString};
use std::path::PathBuf;
use std::ptr;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use eframe::egui;
use eframe::glow::{self, HasContext};
use libloading::Library;
use libloading::os::windows::{Library as WindowsLibrary, LOAD_LIBRARY_SEARCH_DEFAULT_DIRS, LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR, LOAD_LIBRARY_SEARCH_SYSTEM32};

type Handle = c_void;
type Wakeup = unsafe extern "C" fn(*mut c_void);

const FORMAT_STRING: c_int = 1;
const FORMAT_FLAG: c_int = 3;
const FORMAT_INT64: c_int = 4;
const FORMAT_DOUBLE: c_int = 5;
const FORMAT_NODE: c_int = 6;
const FORMAT_NODE_ARRAY: c_int = 7;
const FORMAT_NODE_MAP: c_int = 8;
const EVENT_PROPERTY_CHANGE: c_int = 22;

const WATCH_POSITION: u64 = 1;
const WATCH_DURATION: u64 = 2;
const WATCH_PAUSE: u64 = 3;
const WATCH_SEEKABLE: u64 = 4;
const WATCH_BUFFERING: u64 = 5;
const WATCH_TRACKS: u64 = 6;
const WATCH_BUFFER_POSITION: u64 = 7;
const WATCH_CACHE_SPEED: u64 = 8;
const WATCH_AUDIO: u64 = 9;
const WATCH_SUBTITLE: u64 = 10;

const COMMAND_LOAD: u64 = 1;
const COMMAND_SEEK: u64 = 2;
const COMMAND_SUBTITLE: u64 = 3;

#[repr(C)]
struct RenderParam {
    kind: c_int,
    data: *mut c_void,
}

impl RenderParam {
    fn new<T>(kind: c_int, data: &mut T) -> Self {
        Self { kind, data: ptr::from_mut(data).cast() }
    }
}

#[repr(C)]
struct GlInit {
    get_proc_address: unsafe extern "C" fn(*mut c_void, *const c_char) -> *const c_void,
    context: *mut c_void,
}

#[repr(C)]
struct GlFramebuffer {
    id: c_int,
    width: c_int,
    height: c_int,
    internal_format: c_int,
}

#[repr(C)]
struct RawEvent {
    id: c_int,
    error: c_int,
    reply_userdata: u64,
    data: *const c_void,
}

#[repr(C)]
struct EndFile {
    reason: c_int,
    error: c_int,
}

#[repr(C)]
struct LogMessage {
    prefix: *const c_char,
    level: *const c_char,
    text: *const c_char,
    _log_level: c_int,
}

#[repr(C)]
struct EventProperty {
    _name: *const c_char,
    format: c_int,
    data: *const c_void,
}

#[repr(C)]
union NodeValue {
    string: *const c_char,
    flag: c_int,
    int64: i64,
    list: *const NodeList,
}

#[repr(C)]
struct RawNode {
    value: NodeValue,
    format: c_int,
}

#[repr(C)]
struct NodeList {
    count: c_int,
    values: *const RawNode,
    keys: *const *const c_char,
}

pub enum Event {
    Log(String),
    Loaded,
    SeekStarted,
    SeekFailed,
    Restarted,
    Ended,
    Failed(c_int),
    SettingFailed,
    SubtitleAdded(bool),
    Position(f64),
    Duration(f64),
    Paused(bool),
    Seekable(bool),
    Buffering(bool),
    BufferPosition(f64),
    CacheSpeed(f64),
    Tracks(Vec<Track>),
    SelectedTrack(TrackKind, Option<i64>),
}

#[derive(Clone, Copy, PartialEq)]
pub enum TrackKind {
    Audio,
    Subtitle,
}

pub struct Track {
    pub kind: TrackKind,
    pub id: i64,
    pub stream_index: Option<i64>,
    pub language: Option<String>,
    pub label: String,
    pub selected: bool,
}

struct Api {
    _lib: Library,
    path: String,
    create: unsafe extern "C" fn() -> *mut Handle,
    initialize: unsafe extern "C" fn(*mut Handle) -> c_int,
    load_config_file: unsafe extern "C" fn(*mut Handle, *const c_char) -> c_int,
    terminate_destroy: unsafe extern "C" fn(*mut Handle),
    set_option_string: unsafe extern "C" fn(*mut Handle, *const c_char, *const c_char) -> c_int,
    set_property: unsafe extern "C" fn(*mut Handle, u64, *const c_char, c_int, *mut c_void) -> c_int,
    command: unsafe extern "C" fn(*mut Handle, u64, *const *const c_char) -> c_int,
    wait_event: unsafe extern "C" fn(*mut Handle, f64) -> *const RawEvent,
    observe_property: unsafe extern "C" fn(*mut Handle, u64, *const c_char, c_int) -> c_int,
    request_log_messages: unsafe extern "C" fn(*mut Handle, *const c_char) -> c_int,
    set_wakeup_callback: unsafe extern "C" fn(*mut Handle, Option<Wakeup>, *mut c_void),
    render_create: unsafe extern "C" fn(*mut *mut Handle, *mut Handle, *mut RenderParam) -> c_int,
    render_set_callback: unsafe extern "C" fn(*mut Handle, Option<Wakeup>, *mut c_void),
    render_update: unsafe extern "C" fn(*mut Handle) -> u64,
    render: unsafe extern "C" fn(*mut Handle, *mut RenderParam) -> c_int,
    render_free: unsafe extern "C" fn(*mut Handle),
}

unsafe impl Send for Api {}
unsafe impl Sync for Api {}

static API: OnceLock<Option<Api>> = OnceLock::new();

fn candidates() -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    if let Ok(p) = std::env::var("MPV_DLL") {
        out.push(PathBuf::from(p));
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            out.push(dir.join("libmpv-2.dll"));
            out.push(dir.join("mpv-2.dll"));
            out.push(dir.join("runtime/libmpv-2.dll"));
            out.push(dir.join("runtime/mpv-2.dll"));
        }
    }
    out.push(PathBuf::from("libmpv-2.dll"));
    out.push(PathBuf::from("mpv-2.dll"));
    if let Some(path) = std::env::var_os("PATH") {
        out.extend(std::env::split_paths(&path).filter(|dir| dir.is_absolute())
            .flat_map(|dir| [dir.join("libmpv-2.dll"), dir.join("mpv-2.dll")])
            .filter(|path| path.is_file()));
    }
    out
}

fn api() -> Option<&'static Api> {
    API.get_or_init(|| {
        for path in candidates() {
            let flags = LOAD_LIBRARY_SEARCH_DEFAULT_DIRS
                | if path.is_absolute() { LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR } else { 0 };
            let lib: Library = match unsafe { WindowsLibrary::load_with_flags(&path, flags) } {
                Ok(l) => l.into(),
                Err(_) => continue,
            };
            unsafe {
                macro_rules! sym {
                    ($name:literal, $ty:ty) => {
                        match lib.get::<$ty>($name.as_bytes()) {
                            Ok(s) => *s,
                            Err(_) => continue,
                        }
                    };
                }
                let create = sym!("mpv_create", unsafe extern "C" fn() -> *mut Handle);
                let initialize = sym!("mpv_initialize", unsafe extern "C" fn(*mut Handle) -> c_int);
                let terminate_destroy =
                    sym!("mpv_terminate_destroy", unsafe extern "C" fn(*mut Handle));
                let set_option_string = sym!(
                    "mpv_set_option_string",
                    unsafe extern "C" fn(*mut Handle, *const c_char, *const c_char) -> c_int
                );
                let set_property = sym!(
                    "mpv_set_property_async",
                    unsafe extern "C" fn(*mut Handle, u64, *const c_char, c_int, *mut c_void) -> c_int
                );
                let command = sym!(
                    "mpv_command_async",
                    unsafe extern "C" fn(*mut Handle, u64, *const *const c_char) -> c_int
                );
                let wait_event = sym!("mpv_wait_event", unsafe extern "C" fn(*mut Handle, f64) -> *const RawEvent);
                let observe_property = sym!(
                    "mpv_observe_property",
                    unsafe extern "C" fn(*mut Handle, u64, *const c_char, c_int) -> c_int
                );
                let set_wakeup_callback = sym!(
                    "mpv_set_wakeup_callback",
                    unsafe extern "C" fn(*mut Handle, Option<Wakeup>, *mut c_void)
                );
                return Some(Api {
                    path: path.display().to_string(),
                    create,
                    initialize,
                    load_config_file: sym!("mpv_load_config_file", unsafe extern "C" fn(*mut Handle, *const c_char) -> c_int),
                    terminate_destroy,
                    set_option_string,
                    set_property,
                    command,
                    wait_event,
                    observe_property,
                    request_log_messages: sym!("mpv_request_log_messages", unsafe extern "C" fn(*mut Handle, *const c_char) -> c_int),
                    set_wakeup_callback,
                    render_create: sym!("mpv_render_context_create", unsafe extern "C" fn(*mut *mut Handle, *mut Handle, *mut RenderParam) -> c_int),
                    render_set_callback: sym!("mpv_render_context_set_update_callback", unsafe extern "C" fn(*mut Handle, Option<Wakeup>, *mut c_void)),
                    render_update: sym!("mpv_render_context_update", unsafe extern "C" fn(*mut Handle) -> u64),
                    render: sym!("mpv_render_context_render", unsafe extern "C" fn(*mut Handle, *mut RenderParam) -> c_int),
                    render_free: sym!("mpv_render_context_free", unsafe extern "C" fn(*mut Handle)),
                    _lib: lib,
                });
            }
        }
        None
    })
    .as_ref()
}

fn c(s: &str) -> CString {
    CString::new(s.replace('\0', "")).unwrap_or_default()
}

// The render context is used on the UI's GL thread. A paint callback can
// outlive its player for one frame, so Drop clears its shared slot first.
struct RenderContext(*mut Handle);
unsafe impl Send for RenderContext {}

pub struct Mpv {
    api: &'static Api,
    handle: *mut Handle,
    _wakeup_context: Box<egui::Context>,
    render: Arc<Mutex<Option<RenderContext>>>,
}

impl Mpv {
    pub fn new(ctx: &egui::Context, config: &std::path::Path, options: &[(&str, String)], log: bool)
        -> Result<(Self, Vec<String>), String>
    {
        let api = api().ok_or("The libmpv playback engine could not be initialized")?;
        eprintln!("libmpv: {}", api.path);
        static OPENGL: OnceLock<Option<WindowsLibrary>> = OnceLock::new();
        let gl = OPENGL.get_or_init(|| unsafe {
            WindowsLibrary::load_with_flags("opengl32.dll", LOAD_LIBRARY_SEARCH_SYSTEM32).ok()
        }).as_ref().ok_or("OpenGL could not be initialized")?;

        unsafe extern "C" fn get_proc(context: *mut c_void, name: *const c_char) -> *const c_void {
            type GetProc = unsafe extern "system" fn(*const c_char) -> *const c_void;
            let gl = unsafe { &*context.cast::<WindowsLibrary>() };
            let proc = unsafe { gl.get::<GetProc>(b"wglGetProcAddress\0") }
                .map_or(ptr::null(), |get| unsafe { get(name) });
            // WGL returns reserved sentinel values for unavailable functions.
            if matches!(proc as usize, 0..=3 | usize::MAX) {
                let name = unsafe { CStr::from_ptr(name) };
                unsafe { gl.get::<unsafe extern "system" fn()>(name.to_bytes_with_nul()) }
                    .map_or(ptr::null(), |symbol| *symbol as *const c_void)
            } else {
                proc
            }
        }

        unsafe extern "C" fn request_update(data: *mut c_void) {
            // mpv calls from its own threads. Wake egui and paint on the GL thread.
            let ctx = unsafe { &*data.cast::<egui::Context>() };
            ctx.request_repaint_after(Duration::from_millis(16));
        }

        unsafe {
            let handle = (api.create)();
            if handle.is_null() {
                return Err("The libmpv playback engine could not be initialized".into());
            }
            if log {
                (api.request_log_messages)(handle, c"info".as_ptr());
            }
            let opts = [
                (c"idle", c"yes"),
                (c"keep-open", c"no"),
                (c"hwdec", c"auto-copy"),
                (c"vo", c"libmpv"),
                (c"ao", c"wasapi"),
                (c"osc", c"no"),
                (c"input-default-bindings", c"no"),
                (c"input-vo-keyboard", c"no"),
                (c"terminal", c"no"),
                (c"really-quiet", c"yes"),
                (c"config", c"no"),
            ];
            for (key, value) in opts {
                (api.set_option_string)(handle, key.as_ptr(), value.as_ptr());
            }
            if config.is_file() {
                let path = c(&config.to_string_lossy());
                if (api.load_config_file)(handle, path.as_ptr()) < 0 {
                    (api.terminate_destroy)(handle);
                    return Err("mpv.conf could not be loaded".into());
                }
            }
            let mut rejected = Vec::new();
            for (key, value) in options {
                let (key, value) = (c(key), c(value));
                let result = (api.set_option_string)(handle, key.as_ptr(), value.as_ptr());
                if result < 0 {
                    eprintln!("mpv option {} was rejected: {result}", key.to_string_lossy());
                    rejected.push(key.to_string_lossy().into_owned());
                }
            }
            (api.set_option_string)(handle, c"vo".as_ptr(), c"libmpv".as_ptr());
            (api.set_option_string)(handle, c"osc".as_ptr(), c"no".as_ptr());
            if (api.initialize)(handle) < 0 {
                (api.terminate_destroy)(handle);
                return Err("The libmpv playback engine could not be initialized".into());
            }
            for (id, name, format) in [
                (WATCH_POSITION, c"time-pos", FORMAT_DOUBLE),
                (WATCH_DURATION, c"duration", FORMAT_DOUBLE),
                (WATCH_PAUSE, c"pause", FORMAT_FLAG),
                (WATCH_SEEKABLE, c"seekable", FORMAT_FLAG),
                (WATCH_BUFFERING, c"paused-for-cache", FORMAT_FLAG),
                (WATCH_TRACKS, c"track-list", FORMAT_NODE),
                (WATCH_AUDIO, c"aid", FORMAT_INT64),
                (WATCH_SUBTITLE, c"sid", FORMAT_INT64),
                (WATCH_BUFFER_POSITION, c"demuxer-cache-time", FORMAT_DOUBLE),
                (WATCH_CACHE_SPEED, c"cache-speed", FORMAT_DOUBLE),
            ] {
                (api.observe_property)(handle, id, name.as_ptr(), format);
            }
            let mut render = ptr::null_mut();
            let mut init = GlInit { get_proc_address: get_proc, context: ptr::from_ref(gl).cast_mut().cast() };
            let mut params = [
                RenderParam { kind: 1, data: c"opengl".as_ptr().cast_mut().cast() }, // API_TYPE
                RenderParam::new(2, &mut init), // OPENGL_INIT_PARAMS
                RenderParam { kind: 0, data: ptr::null_mut() },
            ];
            let result = (api.render_create)(&mut render, handle, params.as_mut_ptr());
            if result < 0 {
                (api.terminate_destroy)(handle);
                return Err(format!("OpenGL video renderer could not be initialized ({result})"));
            }
            let mut context = Box::new(ctx.clone());
            let data = ptr::from_mut(context.as_mut()).cast();
            (api.set_wakeup_callback)(handle, Some(request_update), data);
            (api.render_set_callback)(render, Some(request_update), data);
            Ok((Self {
                api,
                handle,
                _wakeup_context: context,
                render: Arc::new(Mutex::new(Some(RenderContext(render)))),
            }, rejected))
        }
    }

    pub fn paint(&self, painter: &egui::Painter, rect: egui::Rect) {
        let render = Arc::clone(&self.render);
        let api = self.api;
        painter.add(egui::PaintCallback {
            rect,
            callback: Arc::new(eframe::egui_glow::CallbackFn::new(move |info, painter| {
                let render = render.lock().unwrap();
                let Some(RenderContext(render)) = *render else { return };
                let fbo = painter.intermediate_fbo();
                let mut framebuffer = GlFramebuffer {
                    id: fbo.map_or(0, |fbo| fbo.0.get() as c_int),
                    width: info.screen_size_px[0] as c_int,
                    height: info.screen_size_px[1] as c_int,
                    internal_format: 0,
                };
                let mut flip_y: c_int = 1;
                let mut block: c_int = 0;
                let mut params = [
                    RenderParam::new(3, &mut framebuffer), // OPENGL_FBO
                    RenderParam::new(4, &mut flip_y), // FLIP_Y
                    RenderParam::new(12, &mut block), // BLOCK_FOR_TARGET_TIME
                    RenderParam { kind: 0, data: ptr::null_mut() },
                ];
                let gl = painter.gl();
                unsafe {
                    gl.disable(glow::SCISSOR_TEST);
                    (api.render_update)(render);
                    (api.render)(render, params.as_mut_ptr());
                    gl.bind_framebuffer(glow::FRAMEBUFFER, fbo);
                }
                // egui_glow restores the viewport, program, blend and scissor state.
            })),
        });
    }

    // Client commands must never block the GL thread, including a track
    // switch on a paused frame that needs the renderer to wake up.
    fn command(&self, reply: u64, args: &[&str]) -> bool {
        let Ok(args) = args.iter().map(|arg| CString::new(*arg)).collect::<Result<Vec<_>, _>>() else {
            return false;
        };
        let argv: Vec<_> = args.iter().map(|arg| arg.as_ptr()).chain([ptr::null()]).collect();
        unsafe { (self.api.command)(self.handle, reply, argv.as_ptr()) >= 0 }
    }

    pub fn play(&self, url: &str) -> bool {
        self.command(COMMAND_LOAD, &["loadfile", url, "replace"])
    }

    pub fn set_pause(&self, paused: bool) -> bool {
        self.set_property("pause", if paused { "yes" } else { "no" })
    }

    pub fn set_property(&self, name: &str, value: &str) -> bool {
        let (k, v) = (c(name), c(value));
        let mut value = v.as_ptr();
        unsafe { (self.api.set_property)(self.handle, 0, k.as_ptr(), FORMAT_STRING, ptr::from_mut(&mut value).cast()) >= 0 }
    }

    pub fn seek_absolute(&self, secs: f64) -> bool {
        self.command(COMMAND_SEEK, &["seek", &format!("{:.3}", secs.max(0.0)), "absolute+exact"])
    }

    pub fn next_event(&self) -> Option<Event> {
        loop {
            // mpv owns this event until the next call to mpv_wait_event.
            let event = unsafe { &*(self.api.wait_event)(self.handle, 0.0) };
            match event.id {
                0 => return None,
                2 => {
                    let message = unsafe { &*event.data.cast::<LogMessage>() };
                    let value = unsafe { CStr::from_ptr(message.text) }.to_string_lossy();
                    let lower = value.to_ascii_lowercase();
                    if lower.contains("://") || lower.contains("api_key") || lower.contains("x-emby-") {
                        continue;
                    }
                    let prefix = unsafe { CStr::from_ptr(message.prefix) }.to_string_lossy();
                    let level = unsafe { CStr::from_ptr(message.level) }.to_string_lossy();
                    return Some(Event::Log(format!("[{prefix}/{level}] {}", value.trim_end())));
                }
                4 if event.error < 0 => return Some(Event::SettingFailed),
                5 => match event.reply_userdata {
                    COMMAND_LOAD if event.error < 0 => return Some(Event::Failed(event.error)),
                    COMMAND_SEEK if event.error < 0 => return Some(Event::SeekFailed),
                    COMMAND_SUBTITLE => return Some(Event::SubtitleAdded(event.error >= 0)),
                    _ => continue,
                },
                7 => {
                    let end = unsafe { &*event.data.cast::<EndFile>() };
                    match end.reason {
                        0 | 2 | 3 => return Some(Event::Ended),
                        5 => continue, // Playlist redirect: another file is about to load.
                        _ => return Some(Event::Failed(end.error)),
                    }
                }
                8 => return Some(Event::Loaded),
                20 => return Some(Event::SeekStarted),
                21 => return Some(Event::Restarted),
                EVENT_PROPERTY_CHANGE => {
                    // The payload is mpv-owned until the next call to mpv_wait_event.
                    let property = unsafe { &*event.data.cast::<EventProperty>() };
                    // Unavailable properties carry MPV_FORMAT_NONE.
                    if property.data.is_null() {
                        match event.reply_userdata {
                            WATCH_AUDIO => return Some(Event::SelectedTrack(TrackKind::Audio, None)),
                            WATCH_SUBTITLE => return Some(Event::SelectedTrack(TrackKind::Subtitle, None)),
                            WATCH_BUFFER_POSITION => return Some(Event::BufferPosition(0.0)),
                            WATCH_CACHE_SPEED => return Some(Event::CacheSpeed(0.0)),
                            _ => continue,
                        }
                    }
                    let change = match property.format {
                        FORMAT_NODE if event.reply_userdata == WATCH_TRACKS => {
                            let node = unsafe { &*property.data.cast::<RawNode>() };
                            Event::Tracks(unsafe { Self::read_tracks(node) })
                        }
                        FORMAT_INT64 => {
                            let id = unsafe { *property.data.cast::<i64>() };
                            match event.reply_userdata {
                                WATCH_AUDIO => Event::SelectedTrack(TrackKind::Audio, Some(id)),
                                WATCH_SUBTITLE => Event::SelectedTrack(TrackKind::Subtitle, Some(id)),
                                _ => continue,
                            }
                        }
                        FORMAT_DOUBLE => {
                            let value = unsafe { *property.data.cast::<f64>() };
                            if !value.is_finite() || value < 0.0 {
                                continue;
                            }
                            match event.reply_userdata {
                                WATCH_POSITION => Event::Position(value),
                                WATCH_DURATION if value > 0.0 => Event::Duration(value),
                                WATCH_BUFFER_POSITION => Event::BufferPosition(value),
                                WATCH_CACHE_SPEED => Event::CacheSpeed(value),
                                _ => continue,
                            }
                        }
                        FORMAT_FLAG => {
                            let value = unsafe { *property.data.cast::<c_int>() } != 0;
                            match event.reply_userdata {
                                WATCH_PAUSE => Event::Paused(value),
                                WATCH_SEEKABLE => Event::Seekable(value),
                                WATCH_BUFFERING => Event::Buffering(value),
                                _ => continue,
                            }
                        }
                        _ => continue,
                    };
                    return Some(change);
                }
                _ => {}
            }
        }
    }

    /// The event owns all node data until the next call to mpv_wait_event.
    unsafe fn read_tracks(node: &RawNode) -> Vec<Track> {
        if node.format != FORMAT_NODE_ARRAY {
            return Vec::new();
        }
        let list = unsafe { &*node.value.list };
        if list.count <= 0 {
            return Vec::new();
        }
        let nodes = unsafe { std::slice::from_raw_parts(list.values, list.count as usize) };
        nodes.iter().filter_map(|node| {
            if node.format != FORMAT_NODE_MAP {
                return None;
            }
            let list = unsafe { &*node.value.list };
            if list.count <= 0 {
                return None;
            }
            let keys = unsafe { std::slice::from_raw_parts(list.keys, list.count as usize) };
            let values = unsafe { std::slice::from_raw_parts(list.values, list.count as usize) };
            let mut kind = None;
            let mut id = None;
            let mut stream_index = None;
            let mut selected = false;
            let (mut title, mut language, mut codec) = (None, None, None);
            for (&key, node) in keys.iter().zip(values) {
                let name = unsafe { CStr::from_ptr(key) }.to_bytes();
                match (name, node.format) {
                    (b"type", FORMAT_STRING) => {
                        kind = match unsafe { CStr::from_ptr(node.value.string) }.to_bytes() {
                            b"audio" => Some(TrackKind::Audio),
                            b"sub" => Some(TrackKind::Subtitle),
                            _ => None,
                        };
                    }
                    (b"id", FORMAT_INT64) => id = Some(unsafe { node.value.int64 }),
                    (b"ff-index", FORMAT_INT64) => stream_index = Some(unsafe { node.value.int64 }),
                    (b"selected", FORMAT_FLAG) => selected = unsafe { node.value.flag } != 0,
                    (b"title" | b"lang" | b"codec", FORMAT_STRING) => {
                        let text = unsafe { CStr::from_ptr(node.value.string) }.to_string_lossy();
                        match name {
                            b"title" => title = Some(text),
                            b"lang" => language = Some(text),
                            _ => codec = Some(text),
                        }
                    }
                    _ => {}
                }
            }
            let kind = kind?;
            let id = id?;
            let lang = language.as_ref().map(|value| value.to_string());
            let parts: Vec<_> = [title, language, codec].into_iter().flatten()
                .filter(|part| !part.is_empty()).collect();
            let label = if parts.is_empty() { format!("#{id}") } else { parts.join(" · ") };
            Some(Track { kind, id, stream_index, language: lang, label, selected })
        }).collect()
    }

    pub fn select_track(&self, kind: TrackKind, id: Option<i64>) -> bool {
        let property = match kind {
            TrackKind::Audio => "aid",
            TrackKind::Subtitle => "sid",
        };
        let value = id.map_or_else(|| "no".to_string(), |id| id.to_string());
        self.set_property(property, &value)
    }

    pub fn add_subtitle(&self, url: &str, title: Option<&str>) -> bool {
        let mut args = vec!["sub-add", url, "select"];
        if let Some(title) = title {
            args.push(title);
        }
        self.command(COMMAND_SUBTITLE, &args)
    }
}

impl Drop for Mpv {
    fn drop(&mut self) {
        unsafe {
            (self.api.set_wakeup_callback)(self.handle, None, ptr::null_mut());
            if let Some(RenderContext(render)) = self.render.lock().unwrap().take() {
                (self.api.render_set_callback)(render, None, ptr::null_mut());
                (self.api.render_free)(render);
            }
            (self.api.terminate_destroy)(self.handle);
        }
    }
}
