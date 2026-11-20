//! Two languages, switched while the app runs.
//!
//! Every visible label is written in English in the source and looked up here.
//! A missing entry falls back to the English text, so nothing ever disappears:
//! it just stays in English until a translation is added.

use std::borrow::Cow;
use std::sync::atomic::{AtomicU8, Ordering};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Lang {
    En,
    Zh,
    ZhHant,
}

static CURRENT: AtomicU8 = AtomicU8::new(0);

pub fn set(lang: Lang) {
    CURRENT.store(
        match lang {
            Lang::En => 0,
            Lang::Zh => 1,
            Lang::ZhHant => 2,
        },
        Ordering::Relaxed,
    );
}

pub fn get() -> Lang {
    match CURRENT.load(Ordering::Relaxed) {
        1 => Lang::Zh,
        2 => Lang::ZhHant,
        _ => Lang::En,
    }
}

pub fn is_zh() -> bool {
    get() != Lang::En
}

pub fn code(lang: Lang) -> &'static str {
    match lang {
        Lang::En => "en",
        Lang::Zh => "zh",
        Lang::ZhHant => "zh-Hant",
    }
}

pub fn from_code(text: &str) -> Lang {
    let t = text.trim().to_ascii_lowercase();
    if ["zh-hant", "zh-tw", "zh-hk", "zh-mo", "zh_tw", "zh_hk"].iter().any(|code| t.starts_with(code)) {
        Lang::ZhHant
    } else if t.starts_with("zh") || t.contains("chinese") || t.contains("涓枃") {
        Lang::Zh
    } else {
        Lang::En
    }
}

/// The language to start in when nothing has been chosen: Chinese for a Chinese
/// Windows, English otherwise.
pub fn from_os() -> Lang {
    if let Some(lang) = ["LC_ALL", "LANG", "USERLANGUAGE"].iter()
        .find_map(|key| std::env::var(key).ok().filter(|lang| !lang.is_empty()))
    {
        return from_code(&lang);
    }
    let mut locale = [0u16; 85];
    let len = unsafe { windows_sys::Win32::Globalization::GetUserDefaultLocaleName(locale.as_mut_ptr(), locale.len() as i32) };
    if len <= 1 { Lang::En } else { from_code(&String::from_utf16_lossy(&locale[..len as usize - 1])) }
}

pub fn bilingual<'a>(english: &'a str, chinese: &'a str) -> Cow<'a, str> {
    match get() {
        Lang::En => Cow::Borrowed(english),
        Lang::Zh => Cow::Borrowed(chinese),
        Lang::ZhHant => {
            use windows_sys::Win32::Globalization::{LCMapStringEx, LCMAP_TRADITIONAL_CHINESE};
            let input: Vec<u16> = chinese.encode_utf16().collect();
            let mut output = vec![0u16; input.len() * 2];
            let length = unsafe {
                LCMapStringEx([0u16].as_ptr(), LCMAP_TRADITIONAL_CHINESE,
                    input.as_ptr(), input.len() as i32, output.as_mut_ptr(), output.len() as i32,
                    std::ptr::null(), std::ptr::null(), 0)
            };
            if length > 0 { Cow::Owned(String::from_utf16_lossy(&output[..length as usize])) }
            else { Cow::Borrowed(chinese) }
        }
    }
}

/// Translate one label. English is returned untouched.
pub fn t(text: &str) -> Cow<'_, str> {
    if !is_zh() {
        return Cow::Borrowed(text);
    }
    match lookup(text) {
        Some(zh) => bilingual(text, zh),
        None => {
            // A miss list is a developer aid only: it prints when the environment
            // variable is set (tools\zh_all.ps1 does), so a normal run stays quiet.
            if std::env::var_os("MEDIA_LITE_I18N_DEBUG").is_some()
                && text.is_ascii()
                && text.chars().any(|c| c.is_ascii_alphabetic())
            {
                eprintln!("I18N-MISS {}", text);
            }
            Cow::Borrowed(text)
        }
    }
}

/// Translate a longer sentence, including the messages the server layer builds.
pub fn sentence(text: &str) -> String {
    if !is_zh() {
        return text.to_string();
    }
    if let Some(zh) = lookup(text) {
        return zh.to_string();
    }
    // The user-facing status is "what: message"; translate the two halves.
    if let Some((what, rest)) = text.split_once(": ") {
        if let Some(w) = lookup(what) {
            return format!("{}: {}", w, sentence(rest));
        }
    }
    // "the server answered 401 Unauthorized"
    if let Some(rest) = text.strip_prefix("the server answered ") {
        return format!("服务器返回 {}", rest);
    }
    text.to_string()
}

/// A label with a number in it, e.g. "12 items".
pub fn counted(n: i64, one: &str, many: &str) -> String {
    if is_zh() {
        match lookup(many) {
            Some(zh) => format!("{} {}", n, zh),
            None => format!("{} {}", n, many),
        }
    } else {
        format!("{} {}", n, one)
    }
}

fn lookup(text: &str) -> Option<&'static str> {
    TABLE
        .iter()
        .find(|(en, _)| *en == text)
        .map(|(_, zh)| *zh)
}

/// English 鈫?Chinese. Keep the English side identical to the text in the code.
static TABLE: &[(&str, &str)] = &[
    // ---------------------------------------------------------- navigation
    ("Home", "首页"),
    ("Search", "搜索"),
    ("Settings", "设置"),
    ("Favourites", "收藏"),
    ("Favorites", "收藏"),
    ("Libraries", "媒体库"),
    ("Library", "媒体库"),
    ("Media Library", "媒体库"),
    ("Servers", "服务器"),
    ("Server", "服务器"),
    ("Player", "播放器"),
    ("Appearance", "外观"),
    ("Language", "语言"),
    ("About", "关于"),
    ("Sort", "排序"),
    ("Name", "名称"),
    ("Newest", "最近添加"),
    ("Rating", "评分"),
    ("Filter", "筛选"),
    ("All", "全部"),
    ("Switch to List", "切换成列表"),
    ("Switch to Grid", "切换成海报墙"),
    ("Small", "小"),
    ("Medium", "中"),
    ("Large", "大"),
    ("result", "个结果"),
    ("results", "个结果"),
    ("Watched today", "今天看过"),
    ("Watched yesterday", "昨天看过"),
    // ------------------------------------------------------------- welcome
    (
        "A desktop player for your Emby server. Add as many servers as you like \u{2014} there is no limit and no paid tier.",
        "A desktop player for your Emby server. Add as many servers as you like \u{2014} there is no limit and no paid tier.",
    ),
    ("Add server\u{2026}", "Add server…"),
    ("Add a server\u{2026}", "Add a server…"),
    ("Preview with a sample library", "用示例库预览"),
    ("Unlimited servers", "无限服务器"),
    (
        "Every server keeps its own sign-in, so work, home and a friend's library all live in the sidebar.",
        "Every server keeps its own sign-in, so work, home and a friend's library all live in the sidebar.",
    ),
    ("Continue where you stopped", "从上次停下的地方继续"),
    (
        "Resume points are read from and written back to each server, so the phone and this window agree.",
        "Resume points are read from and written back to each server, so the phone and this window agree.",
    ),
    ("Open in the system player", "用系统播放器打开"),
    (
        "This window draws the film's artwork; the stream itself is sent to the system video player.",
        "This window draws the film's artwork; the stream itself is sent to the system video player.",
    ),
    // -------------------------------------------------------------- dialog
    ("Add a server", "添加服务器"),
    ("Sign in to this server", "登录这台服务器"),
    (
        "The address of your Emby server, then the account to use. Emby usually listens on port 8096.",
        "The address of your Emby server, then the account to use. Emby usually listens on port 8096.",
    ),
    ("Server address", "服务器地址"),
    ("User name", "用户名"),
    ("Password", "密码"),
    ("No server is selected", "未选择服务器"),
    ("LIBRARIES", "媒体库"),
    ("Settings are stored in", "设置保存在"),
    ("Media Lite 1.0 鈥?a desktop Emby client built from a study of an existing player. It speaks the Emby API directly: libraries, resume points, watched flags, favourites, search and playback progress all live on your server.", "Media Lite 1.0 鈥?a desktop Emby client built from a study of an existing player. It speaks the Emby API directly: libraries, resume points, watched flags, favourites, search and playback progress all live on your server."),
    ("Sign in", "登录"),
    ("Signing in\u{2026}", "Signing in…"),
    ("Cancel", "取消"),
    ("Remove", "移除"),
    ("Sign out", "退出登录"),
    ("Reconnect", "重新连接"),
    ("Save", "保存"),
    ("Connect", "连接"),
    ("Connected", "已连接"),
    ("Edit", "编辑"),
    ("Refresh", "刷新"),
    ("Favourite", "收藏"),
    ("Favourited", "已收藏"),
    ("Watched", "已看"),
    ("sample library", "示例库"),
    ("signed in as", "已登录为"),
    ("Type the server's address first.", "请先填写服务器地址。"),
    ("Type the user name to sign in with.", "请填写要登录的用户名。"),
    ("Remove this server?", "移除这台服务器？"),
    // ---------------------------------------------------------------- home
    ("Loading your library\u{2026}", "正在加载媒体库…"),
    ("Loading\u{2026}", "Loading…"),
    ("Continue Watching", "继续观看"),
    ("Next Up", "接下来播放"),
    ("Recently Added", "最近添加"),
    ("See all", "查看全部"),
    ("Could not load the library", "媒体库加载失败"),
    ("Previous page", "上一页"),
    ("Next page", "下一页"),
    ("Nothing here yet", "这里还没有内容"),
    (
        "Add media to this server, or switch to another server in the sidebar.",
        "Add media to this server, or switch to another server in the sidebar.",
    ),
    ("Play", "播放"),
    ("Resume", "继续播放"),
    ("Details", "详情"),
    ("View details", "查看详情"),
    ("Add a server to see libraries", "添加服务器以查看媒体库"),
    // -------------------------------------------------------------- search
    ("Search this server", "搜索这台服务器"),
    ("Search every library", "搜索所有媒体库"),
    ("Searching\u{2026}", "Searching…"),
    (
        "One box searches the server you are on, including episodes and collections.",
        "One box searches the server you are on, including episodes and collections.",
    ),
    ("Type a name and press Enter.", "输入名称后按回车。"),
    ("Nothing matched that name.", "没有找到匹配的内容。"),
    // ------------------------------------------------------------- library
    ("No items in this library yet.", "这个媒体库还没有内容。"),
    ("This library is empty", "这个媒体库是空的"),
    ("items", "个项目"),
    ("item", "个项目"),
    // -------------------------------------------------------------- detail
    ("Overview", "简介"),
    ("Episodes", "剧集"),
    ("Back", "返回"),
    ("Minimize", "最小化"),
    ("Maximize", "最大化"),
    ("Restore window", "还原窗口"),
    ("More", "更多"),
    ("Refresh details", "刷新详情"),
    ("Play from start", "从头播放"),
    ("Present", "现在"),
    ("Version", "版本"),
    ("Audio", "音频"),
    ("Unavailable", "不可用"),
    ("Automatic", "自动"),
    ("Loading media options…", "正在加载媒体选项…"),
    ("Episode layout", "切换剧集布局"),
    ("Go to episode", "选集"),
    ("No episodes found", "没有找到剧集"),
    ("Cast and crew", "演职人员"),
    ("Seasons", "季"),
    ("More like this", "相似内容"),
    ("In this collection", "此合集内"),
    ("Mark watched", "标记为已看"),
    ("Mark unwatched", "标记为未看"),
    ("Add to favourites", "加入收藏"),
    ("In favourites", "已收藏"),
    ("That item could not be loaded.", "无法加载该内容。"),
    ("Finished \u{2014} marked as watched", "Finished — marked as watched"),
    ("Playing S", "Playing S"),
    // ------------------------------------------------------------ settings
    (
        "Add as many Emby servers as you like. Each one keeps its own sign-in; switching is a click in the sidebar.",
        "Add as many Emby servers as you like. Each one keeps its own sign-in; switching is a click in the sidebar.",
    ),
    ("Dark theme", "Dark theme"),
    ("Light theme", "Light theme"),
    ("Use the dark theme", "Use the dark theme"),
    ("Settings are stored in {}", "Settings are stored in {}"),
    ("Signed in", "Signed in"),
    ("Signed out", "Signed out"),
    ("not signed in", "not signed in"),
    ("Fullscreen player", "Fullscreen player"),
    ("Launch on the player screen", "Launch on the player screen"),
    // -------------------------------------------------------------- player
    ("Now playing", "Now playing"),
    ("Playing \u{2014} progress reported to the server", "Playing — progress reported to the server"),
    ("Paused", "Paused"),
    ("Close", "关闭"),
    ("Pause", "暂停"),
    ("Loading video…", "正在加载视频…"),
    ("Resuming playback…", "正在恢复播放位置…"),
    ("Buffering…", "正在缓冲…"),
    ("Offline preview", "离线演示"),
    ("Playback ended", "播放结束"),
    ("Replay", "重播"),
    ("Previous episode", "上一集"),
    ("Next episode", "下一集"),
    ("Always on top", "窗口置顶"),
    ("Minimize", "最小化"),
    ("Maximize", "最大化"),
    ("Restore", "还原"),
    ("Exit fullscreen", "退出全屏"),
    ("Playlist", "播放列表"),
    ("Loading episodes…", "正在加载选集…"),
    ("Danmaku", "弹幕"),
    ("Search danmaku", "搜索弹幕"),
    ("Search subtitles", "搜索字幕 · SubtitleCat"),
    ("Show danmaku", "显示弹幕"),
    ("Import local danmaku XML", "导入本地弹幕 XML"),
    ("Danmaku loaded", "弹幕已载入"),
    ("Could not read danmaku XML", "无法读取弹幕 XML"),
    ("Open subtitle file", "打开字幕文件"),
    ("Subtitle file loaded", "字幕文件已载入"),
    ("Could not load subtitle file", "无法加载字幕文件"),
    ("Could not seek in this media", "无法跳转到该位置"),
    ("Could not apply playback setting", "无法应用播放设置"),
    ("Fit video", "等比适应"),
    ("Fill window", "填满窗口"),
    ("Show cache speed", "显示缓冲速度"),
    ("Automatically hide controls", "自动隐藏控件"),
    ("OpenGL could not be initialized", "无法初始化 OpenGL"),
    ("Fullscreen", "全屏"),
    ("Volume", "音量"),
    ("Muted", "已静音"),
    ("Playback speed", "播放倍速"),
    ("Mute", "静音"),
    ("Unmute", "取消静音"),
    ("Audio track", "音轨"),
    ("Subtitle track", "字幕"),
    ("Turn off subtitles", "关闭字幕"),
    ("No tracks available", "没有可用轨道"),
    ("Next episode", "Next episode"),
    ("Previous episode", "Previous episode"),
    // -------------------------------------------------------------- status
    ("login", "login"),
    ("connect", "connect"),
    ("probe", "probe"),
    ("libraries", "libraries"),
    ("home", "home"),
    ("library", "library"),
    ("search", "search"),
    ("details", "details"),
    ("episodes", "episodes"),
    ("watched", "watched"),
    ("favourite", "favourite"),
    ("progress", "progress"),
    ("favorites", "favorites"),
    (
        "could not reach the server \u{2014} check the address and that the server is running",
        "could not reach the server \u{2014} check the address and that the server is running",
    ),
    ("the server did not accept that user name and password", "the server did not accept that user name and password"),
    ("the server took too long to answer", "the server took too long to answer"),
    ("the server sent something that is not JSON", "the server sent something that is not JSON"),
    ("the answer from the server could not be read", "the answer from the server could not be read"),
    ("nothing answered at that address", "nothing answered at that address"),
    // --------------------------------------------------------------- kinds
    ("Movie", "电影"),
    ("Series", "剧集"),
    ("Episode", "单集"),
    ("Season", "Season"),
    ("BoxSet", "BoxSet"),
    ("Video", "Video"),
    ("Trailer", "Trailer"),
    ("MusicVideo", "MusicVideo"),
    ("Person", "Person"),
    ("Movies", "Movies"),
    ("TV Shows", "TV Shows"),
    ("Collections", "Collections"),
    ("Sample Library", "Sample Library"),
    ("Sample library", "Sample library"),
    ("sample library", "示例库"),
    // ------------------------------------------------------- restored labels
    ("Aggregate", "聚合视界"),
    ("Favourite films", "收藏的电影"),
    ("Favourite series", "收藏的剧"),
    // ------------------------------------------------------ settings labels
    ("General", "常规"),
    ("Backup & restore", "备份与还原"),
    ("Sync", "同步"),
    ("Network", "网络"),
    ("Defaults & interaction", "默认与交互"),
    ("Playback", "播放配置"),
    ("Playback cache", "播放缓存"),
    ("Subtitles", "字幕样式"),
    ("External player", "外部播放器"),
    ("Danmaku", "弹幕"),
    ("Account", "账号"),
    ("Not signed in", "未登录"),
    ("Unlock more features", "解锁更多功能"),
    ("Theme", "主题模式"),
    ("Change the current theme", "修改当前主题模式"),
    ("Open on start", "App启动时打开"),
    ("Minimise to tray on close", "关闭时最小化到托盘"),
    ("Closing the window keeps the app running in the tray", "关闭主界面时将程序最小化到系统托盘"),
    ("Dark", "黑暗"),
    ("Light", "明亮"),
    ("Server settings", "服务器设置"),
    ("Auto", "Auto"),
    ("Open on start / Home", "首页"),
];
