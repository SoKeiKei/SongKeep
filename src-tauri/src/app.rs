use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    process::Command,
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

use base64::{Engine, engine::general_purpose::STANDARD as BASE64};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_dialog::DialogExt;
use uuid::Uuid;

use crate::{
    audio,
    suno::{self, Progress, Song},
};

#[derive(Default)]
struct Session {
    current_song: Option<Song>,
    loaded_url: Option<String>,
    last_downloaded: Option<PathBuf>,
    preview_cache: Option<(String, Vec<u8>)>,
}

#[derive(Default)]
pub struct AppState {
    session: Mutex<Session>,
    downloading: AtomicBool,
}

struct DownloadGuard<'a>(&'a AtomicBool);

impl Drop for DownloadGuard<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadOptions {
    url: String,
    format: Option<String>,
    save_lyrics: Option<bool>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Preferences {
    download_folder: PathBuf,
}

fn error_response(error: impl std::fmt::Display) -> Value {
    json!({ "ok": false, "error": error.to_string() })
}

fn canceled_response() -> Value {
    json!({ "ok": false, "canceled": true })
}

fn preferences_path(app: &AppHandle) -> anyhow::Result<PathBuf> {
    Ok(app.path().app_config_dir()?.join("preferences.json"))
}

fn system_download_folder(app: &AppHandle) -> anyhow::Result<PathBuf> {
    app.path().download_dir().map_err(Into::into)
}

fn configured_download_folder(app: &AppHandle) -> anyhow::Result<Option<PathBuf>> {
    let path = preferences_path(app)?;
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let saved: Preferences = match serde_json::from_str(&text) {
        Ok(saved) => saved,
        Err(_) => return Ok(None),
    };
    if saved.download_folder.is_absolute() && saved.download_folder.is_dir() {
        Ok(Some(saved.download_folder))
    } else {
        Ok(None)
    }
}

fn download_folder_state(app: &AppHandle) -> anyhow::Result<(PathBuf, bool)> {
    if let Some(folder) = configured_download_folder(app)? {
        Ok((folder, true))
    } else {
        Ok((system_download_folder(app)?, false))
    }
}

fn save_preferences(app: &AppHandle, folder: &Path) -> anyhow::Result<()> {
    if !folder.is_absolute() || !folder.is_dir() {
        anyhow::bail!("请选择一个有效的文件夹。");
    }
    let path = preferences_path(app)?;
    let directory = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("无法确定设置目录"))?;
    fs::create_dir_all(directory)?;
    let temporary = directory.join(format!("preferences-{}.tmp", Uuid::new_v4()));
    let data = serde_json::to_vec(&Preferences {
        download_folder: folder.to_path_buf(),
    })?;
    let result = (|| -> anyhow::Result<()> {
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)?;
        file.write_all(&data)?;
        file.sync_all()?;
        drop(file);
        if path.exists() {
            fs::remove_file(&path)?;
        }
        fs::rename(&temporary, &path)?;
        Ok(())
    })();
    let _ = fs::remove_file(&temporary);
    result
}

#[tauri::command]
pub async fn load_song(url: String, state: State<'_, AppState>) -> Result<Value, String> {
    if state.downloading.load(Ordering::Acquire) {
        return Ok(error_response("请等待当前下载完成。"));
    }
    let normalized = url.trim().to_string();
    if normalized.is_empty() {
        return Ok(error_response("请输入有效的 Suno 分享链接。"));
    }
    {
        let mut session = state
            .session
            .lock()
            .map_err(|_| "应用状态读取失败".to_string())?;
        session.current_song = None;
        session.loaded_url = None;
        session.last_downloaded = None;
        session.preview_cache = None;
    }
    match suno::load_song(&normalized).await {
        Ok(song) => {
            let public = song.public();
            let mut session = state
                .session
                .lock()
                .map_err(|_| "应用状态保存失败".to_string())?;
            session.current_song = Some(song);
            session.loaded_url = Some(normalized);
            Ok(json!({ "ok": true, "song": public }))
        }
        Err(error) => Ok(error_response(error)),
    }
}

#[tauri::command]
pub async fn load_preview_audio(state: State<'_, AppState>) -> Result<Value, String> {
    let song = {
        let session = state
            .session
            .lock()
            .map_err(|_| "应用状态读取失败".to_string())?;
        if let (Some(song), Some((cached_id, bytes))) =
            (&session.current_song, &session.preview_cache)
            && cached_id == &song.id
        {
            return Ok(
                json!({ "ok": true, "dataUrl": format!("data:audio/mpeg;base64,{}", BASE64.encode(bytes)) }),
            );
        }
        match &session.current_song {
            Some(song) => song.clone(),
            None => return Ok(error_response("请先读取歌曲。")),
        }
    };
    let clear = match suno::load_audio(&song, &|_| {}).await {
        Ok(bytes) => bytes,
        Err(error) => return Ok(error_response(error)),
    };
    let preview =
        match tauri::async_runtime::spawn_blocking(move || audio::m4a_to_mp3(&clear)).await {
            Ok(Ok(bytes)) => bytes,
            Ok(Err(error)) => return Ok(error_response(error)),
            Err(error) => return Ok(error_response(format!("试听音频准备失败：{error}"))),
        };
    let data_url = format!("data:audio/mpeg;base64,{}", BASE64.encode(&preview));
    let mut session = state
        .session
        .lock()
        .map_err(|_| "应用状态保存失败".to_string())?;
    if session
        .current_song
        .as_ref()
        .is_some_and(|current| current.id == song.id)
    {
        session.preview_cache = Some((song.id, preview));
    }
    Ok(json!({ "ok": true, "dataUrl": data_url }))
}

#[tauri::command]
pub fn copy_lyrics(state: State<'_, AppState>) -> Result<Value, String> {
    let lyrics = state
        .session
        .lock()
        .map_err(|_| "应用状态读取失败".to_string())?
        .current_song
        .as_ref()
        .map(|song| {
            song.lyrics
                .trim_start_matches('\u{feff}')
                .trim()
                .to_string()
        })
        .unwrap_or_default();
    if lyrics.is_empty() {
        return Ok(error_response("当前歌曲没有可复制的歌词。"));
    }
    match arboard::Clipboard::new().and_then(|mut clipboard| clipboard.set_text(lyrics)) {
        Ok(()) => Ok(json!({ "ok": true })),
        Err(error) => Ok(error_response(format!("复制歌词失败：{error}"))),
    }
}

#[tauri::command]
pub fn get_download_folder(app: AppHandle) -> Value {
    match download_folder_state(&app) {
        Ok((folder, is_custom)) => {
            json!({ "ok": true, "folder": folder.to_string_lossy(), "isCustom": is_custom })
        }
        Err(error) => error_response(error),
    }
}

#[tauri::command]
pub async fn choose_download_folder(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Value, String> {
    if state.downloading.load(Ordering::Acquire) {
        return Ok(error_response("请等待当前下载完成。"));
    }
    let (current, _) = match download_folder_state(&app) {
        Ok(value) => value,
        Err(error) => return Ok(error_response(error)),
    };
    let selected = app
        .dialog()
        .file()
        .set_title("设置默认下载位置")
        .set_directory(&current)
        .blocking_pick_folder();
    let Some(selected) = selected else {
        return Ok(canceled_response());
    };
    let folder = match selected.into_path() {
        Ok(folder) => folder,
        Err(error) => return Ok(error_response(error)),
    };
    match save_preferences(&app, &folder) {
        Ok(()) => Ok(json!({ "ok": true, "folder": folder.to_string_lossy(), "isCustom": true })),
        Err(error) => Ok(error_response(error)),
    }
}

#[tauri::command]
pub fn reset_download_folder(app: AppHandle, state: State<'_, AppState>) -> Value {
    if state.downloading.load(Ordering::Acquire) {
        return error_response("请等待当前下载完成。");
    }
    let result = preferences_path(&app).and_then(|path| match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    });
    match result.and_then(|_| system_download_folder(&app)) {
        Ok(folder) => json!({ "ok": true, "folder": folder.to_string_lossy(), "isCustom": false }),
        Err(error) => error_response(error),
    }
}

fn with_extension(mut path: PathBuf, extension: &str) -> PathBuf {
    path.set_extension(extension);
    path
}

fn lyrics_path(audio_path: &Path) -> PathBuf {
    with_extension(audio_path.to_path_buf(), "txt")
}

struct StagedFile {
    destination: PathBuf,
    temporary: PathBuf,
    backup: Option<PathBuf>,
    committed: bool,
}

fn save_files(files: &[(&Path, &[u8])]) -> anyhow::Result<()> {
    let mut staged = Vec::with_capacity(files.len());
    let result = (|| -> anyhow::Result<()> {
        for (destination, data) in files {
            let directory = destination
                .parent()
                .ok_or_else(|| anyhow::anyhow!("无法确定保存目录"))?;
            fs::create_dir_all(directory)?;
            let file_name = destination
                .file_name()
                .ok_or_else(|| anyhow::anyhow!("保存文件名无效"))?
                .to_string_lossy();
            let temporary = directory.join(format!(".{file_name}.{}.songkeep-tmp", Uuid::new_v4()));
            let mut file = OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&temporary)?;
            file.write_all(data)?;
            file.sync_all()?;
            staged.push(StagedFile {
                destination: destination.to_path_buf(),
                temporary,
                backup: None,
                committed: false,
            });
        }
        for file in &mut staged {
            if file.destination.exists() {
                let directory = file
                    .destination
                    .parent()
                    .ok_or_else(|| anyhow::anyhow!("无法确定保存目录"))?;
                let file_name = file
                    .destination
                    .file_name()
                    .ok_or_else(|| anyhow::anyhow!("保存文件名无效"))?
                    .to_string_lossy();
                let backup =
                    directory.join(format!(".{file_name}.{}.songkeep-backup", Uuid::new_v4()));
                fs::rename(&file.destination, &backup)?;
                file.backup = Some(backup);
            }
        }
        for file in &mut staged {
            fs::rename(&file.temporary, &file.destination)?;
            file.committed = true;
        }
        Ok(())
    })();

    if result.is_err() {
        for file in staged.iter().rev() {
            if file.committed {
                let _ = fs::remove_file(&file.destination);
            }
            if let Some(backup) = &file.backup {
                let _ = fs::rename(backup, &file.destination);
            }
        }
    }
    for file in &staged {
        let _ = fs::remove_file(&file.temporary);
        if result.is_ok()
            && let Some(backup) = &file.backup
        {
            let _ = fs::remove_file(backup);
        }
    }
    result
}

fn save_error_message(error: &anyhow::Error) -> String {
    let io_error = error
        .chain()
        .find_map(|cause| cause.downcast_ref::<std::io::Error>());
    match io_error.map(std::io::Error::kind) {
        Some(std::io::ErrorKind::PermissionDenied) => {
            "保存失败：没有权限写入所选位置，请改存到其他文件夹。".to_string()
        }
        Some(std::io::ErrorKind::StorageFull) => {
            "保存失败：磁盘空间不足，请清理空间或选择其他磁盘。".to_string()
        }
        Some(std::io::ErrorKind::NotFound) => {
            "保存失败：所选文件夹不存在或已经被移动，请重新选择保存位置。".to_string()
        }
        _ => format!("保存失败，请检查文件名、保存位置和磁盘状态：{error}"),
    }
}

#[tauri::command]
pub async fn download_song(
    app: AppHandle,
    state: State<'_, AppState>,
    options: DownloadOptions,
) -> Result<Value, String> {
    if state.downloading.swap(true, Ordering::AcqRel) {
        return Ok(error_response("歌曲正在下载中。"));
    }
    let _guard = DownloadGuard(&state.downloading);
    let url = options.url.trim().to_string();
    if url.is_empty() {
        return Ok(error_response("请输入有效的 Suno 分享链接。"));
    }
    let format = if options.format.as_deref() == Some("m4a") {
        "m4a"
    } else {
        "mp3"
    };

    let (song, reloaded) = {
        let current = state
            .session
            .lock()
            .map_err(|_| "应用状态读取失败".to_string())?;
        if current.loaded_url.as_deref() == Some(&url) {
            (current.current_song.clone(), false)
        } else {
            (None, true)
        }
    };
    let song = match song {
        Some(song) => song,
        None => match suno::load_song(&url).await {
            Ok(song) => {
                let mut session = state
                    .session
                    .lock()
                    .map_err(|_| "应用状态保存失败".to_string())?;
                session.current_song = Some(song.clone());
                session.loaded_url = Some(url.clone());
                session.preview_cache = None;
                song
            }
            Err(error) => return Ok(error_response(error)),
        },
    };

    let (folder, _) = match download_folder_state(&app) {
        Ok(value) => value,
        Err(error) => return Ok(error_response(error)),
    };
    let fallback = format!("suno-{}", &song.id[..8]);
    let default_name = format!("{}.{}", suno::safe_filename(&song.title, &fallback), format);
    let selected = app
        .dialog()
        .file()
        .set_title("保存歌曲")
        .set_directory(&folder)
        .set_file_name(&default_name)
        .add_filter(
            if format == "m4a" {
                "M4A 音频"
            } else {
                "MP3 音频"
            },
            &[format],
        )
        .blocking_save_file();
    let Some(selected) = selected else {
        let mut response = canceled_response();
        if reloaded {
            response["song"] =
                serde_json::to_value(song.public()).map_err(|error| error.to_string())?;
        }
        return Ok(response);
    };
    let audio_path = match selected.into_path() {
        Ok(path) => with_extension(path, format),
        Err(error) => return Ok(error_response(error)),
    };
    let save_lyrics = options.save_lyrics.unwrap_or(false) && !song.lyrics.trim().is_empty();
    let lyrics_path = save_lyrics.then(|| lyrics_path(&audio_path));
    if lyrics_path.as_ref().is_some_and(|path| path.exists()) {
        return Ok(error_response(
            "同名歌词文件已存在，请修改歌曲文件名后重试。",
        ));
    }

    let emit_app = app.clone();
    let clear = match suno::load_audio(&song, &move |progress: Progress| {
        let _ = emit_app.emit("song-progress", progress);
    })
    .await
    {
        Ok(bytes) => bytes,
        Err(error) => return Ok(error_response(error)),
    };
    let audio_data = if format == "m4a" {
        clear
    } else {
        let _ = app.emit(
            "song-progress",
            Progress {
                phase: "转换 MP3".into(),
                message: "正在转换 MP3…".into(),
                received: 0,
                total: clear.len() as u64,
                percent: 80.0,
                format: None,
            },
        );
        match tauri::async_runtime::spawn_blocking(move || audio::m4a_to_mp3(&clear)).await {
            Ok(Ok(bytes)) => bytes,
            Ok(Err(error)) => return Ok(error_response(error)),
            Err(error) => return Ok(error_response(format!("MP3 转换任务失败：{error}"))),
        }
    };

    let lyrics_data = song.lyrics.trim_start_matches('\u{feff}').as_bytes();
    let mut outputs = vec![(audio_path.as_path(), audio_data.as_slice())];
    if let Some(path) = &lyrics_path {
        outputs.push((path.as_path(), lyrics_data));
    }
    if let Err(error) = save_files(&outputs) {
        return Ok(error_response(save_error_message(&error)));
    }
    {
        let mut session = state
            .session
            .lock()
            .map_err(|_| "应用状态保存失败".to_string())?;
        session.last_downloaded = Some(audio_path.clone());
    }
    let _ = app.emit(
        "song-progress",
        Progress {
            phase: "完成".into(),
            message: "下载完成，已保存到本地。".into(),
            received: audio_data.len() as u64,
            total: audio_data.len() as u64,
            percent: 100.0,
            format: Some(format.to_string()),
        },
    );
    Ok(json!({
        "ok": true,
        "result": {
            "audioPath": audio_path.to_string_lossy(),
            "lyricsPath": lyrics_path.as_ref().map(|path| path.to_string_lossy().to_string()),
            "bytes": audio_data.len()
        },
        "song": song.public()
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explains_common_local_save_errors() {
        let permission = anyhow::Error::new(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "denied",
        ));
        assert!(save_error_message(&permission).contains("没有权限"));

        let full = anyhow::Error::new(std::io::Error::new(std::io::ErrorKind::StorageFull, "full"));
        assert!(save_error_message(&full).contains("磁盘空间不足"));

        let missing =
            anyhow::Error::new(std::io::Error::new(std::io::ErrorKind::NotFound, "missing"));
        assert!(save_error_message(&missing).contains("文件夹不存在"));
    }
}

#[tauri::command]
pub fn reveal_download(state: State<'_, AppState>) -> Value {
    let path = match state.session.lock() {
        Ok(session) => session.last_downloaded.clone(),
        Err(_) => return error_response("应用状态读取失败"),
    };
    let Some(path) = path else {
        return error_response("还没有可打开的下载文件。");
    };
    if !path.is_file() {
        return error_response("下载文件已被移动或删除，无法打开所在位置。");
    }
    #[cfg(target_os = "windows")]
    let result = Command::new("explorer.exe")
        .arg("/select,")
        .arg(&path)
        .spawn();
    #[cfg(target_os = "macos")]
    let result = Command::new("open").arg("-R").arg(&path).spawn();
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let result = Command::new("xdg-open")
        .arg(path.parent().unwrap_or_else(|| Path::new(".")))
        .spawn();
    match result {
        Ok(_) => json!({ "ok": true }),
        Err(error) => error_response(format!("无法打开文件所在位置：{error}")),
    }
}
