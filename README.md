# Media Lite

Media Lite 是用 Rust 和 egui 编写的 Windows Emby 客户端。支持多服务器、媒体库浏览、收藏、搜索、续播，以及通过 libmpv 切换音轨和字幕。

## 下载与更新

从最新发布下载 Windows x64 ZIP，解压完整目录并运行 `Media Lite.exe`。发布页同时提供 ZIP 的 SHA-256 校验文件。[下载最新版本](https://github.com/WangYiQian0821/media-lite/releases/latest)

在「设置 → Media Lite」选择「检查更新」，可查看最新稳定版并下载 Windows 便携包。「启动时检查更新」默认开启，可在同一页关闭。升级时将新版解压到新目录，再运行新版程序。

服务器和本机设置存放在 `%APPDATA%\MediaLite\config-v2.json`。首次启动时会从同目录的旧版 `config.json` 导入配置。这个文件可能含有登录令牌，请妥善保管。

## 构建

在 Windows x64 上用 Rust 工具链构建。若使用 MinGW，建议在纯 ASCII 路径下构建。

```powershell
cargo build --release --locked
```

播放需要 Windows x64 的 libmpv 0.42.0。把 `libmpv-2.dll` 放在可执行文件旁的 `runtime` 文件夹，或通过 `MPV_DLL` 指定兼容 DLL。便携包包含所用运行库和第三方许可。[mpv 0.42.0](https://github.com/mpv-player/mpv/releases/tag/v0.42.0)

## 发布新版

更新 `Cargo.toml` 和 `Cargo.lock` 中的版本，构建并打包为 `MediaLite-X.Y.Z-Windows-x64.zip`。在 GitHub Releases 发布对应的 `vX.Y.Z` 标签，附上 ZIP 和 SHA-256 校验文件，并设为最新正式版本。
