# SongKeep · Suno 分享歌曲下载

一个可在 Windows 和 macOS 运行的轻量桌面工具。输入 Suno 分享链接后，可查看歌曲名称、歌词和公开可见的作者、发布日期及互动数据，直接试听，下载 MP3 或原始 M4A 音频。可以设置默认下载位置，保存时仍能临时选择其他目录并修改文件名；需要时也可保存 UTF-8 歌词文本。不需要登录 Suno，也不会读取浏览器中的账号或 Cookie。页面没有公开的数据会显示“暂无数据”。

## 下载

前往 [Releases](https://github.com/SoKeiKei/SongKeep/releases) 下载 Windows 便携版 EXE（双击即用）或 macOS 应用。

## 使用

1. 打开程序，粘贴 `https://suno.com/s/...` 或 `https://suno.com/song/...` 链接。可先点击“读取歌曲”查看信息、试听并一键复制歌词；也可直接点击“下载到本地”，程序会先确认是否读取当前链接。
   使用前请先在浏览器确认当前网络可以正常打开 `https://suno.com`；如无法访问，请检查网络或代理设置。
2. 选择 MP3 或 M4A（默认 MP3）。如需另存歌词，手动勾选“同时保存歌词文本”。
3. 在“默认下载位置”处点击“设置”选择常用目录。程序会记住这个设置，下次打开时继续使用；点击“恢复默认”可以回到系统下载目录。
4. 点击“下载到本地”。保存窗口会从默认目录打开，你仍可以临时选择其他位置并修改文件名。目标文件同名时，选择“覆盖”会替换原文件；完成后可点击“打开所在位置”。

无法公开访问、已删除或播放权限受限的歌曲可能无法读取或下载。工具只处理当前输入的单首歌曲。

## 开发与打包

需要 Node.js、npm 和 Rust。在本目录运行：

```text
npm ci
npm start
```

在 Windows 上运行 `npm run build:win`，生成无需安装、双击即可运行的单文件 EXE：

```text
src-tauri/target/x86_64-pc-windows-msvc/release/songkeep.exe
```

在 macOS 上运行 `npm run build:mac`，生成同时兼容 Apple 芯片和 Intel 芯片的应用。不同系统的成品需在对应系统上制作。当前未配置代码签名，首次打开时系统可能提示确认来源。

运行 `npm test` 检查链接、歌曲信息与下载逻辑。MP3 转换已经内置在单文件程序中，不需要安装额外软件或使用第三方下载服务。

## 注意

工具依赖 Suno 当前分享页提供的播放信息与匿名播放权限。Suno 改动页面或播放方式后可能需要更新。

M4A 仅指平台提供的原始文件格式，不代表无损音频、制作母带或更高音质。下载的音频仅供个人试听。本工具不提供任何版权授权，也不对用户后续使用产生的版权问题负责；请用户自行阅读并遵守 Suno 的版权协议与使用规则。

## 免责声明

本项目为个人开源工具，与 Suno 官方无关，未获其授权或认可。仅用于备份你有权访问的公开分享内容；请勿用于侵权、再分发或商业用途。使用者需自行承担使用风险及法律责任。

## 许可证

[MIT](LICENSE) © 2026 SoKei

---

## English

SongKeep is a lightweight desktop app (Windows / macOS, built with Tauri) that downloads a song (MP3 or original M4A) and its lyrics from a public Suno share link — no sign-in, no browser cookies. Paste a `suno.com/s/...` or `suno.com/song/...` link, preview, then save.

- Build: Node.js + Rust, `npm ci`, `npm start` (dev), `npm run build:win` / `npm run build:mac`, `npm test`.
- Disclaimer: not affiliated with Suno. For personal backup of content you are allowed to access only; you are responsible for complying with Suno's terms and copyright law.
- License: MIT.
