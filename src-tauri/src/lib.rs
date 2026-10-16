mod app;
mod audio;
mod suno;

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(app::AppState::default())
        .invoke_handler(tauri::generate_handler![
            app::load_song,
            app::load_preview_audio,
            app::download_song,
            app::copy_lyrics,
            app::get_download_folder,
            app::choose_download_folder,
            app::reset_download_folder,
            app::reveal_download,
        ])
        .run(tauri::generate_context!())
        .expect("SongKeep 启动失败");
}
