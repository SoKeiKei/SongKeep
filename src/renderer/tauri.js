'use strict';

const tauri = window.__TAURI__;

if (!tauri?.core?.invoke || !tauri?.event?.listen) {
  throw new Error('桌面运行环境初始化失败，请重新启动应用。');
}

const invoke = tauri.core.invoke;

window.sunoApp = Object.freeze({
  loadSong: url => invoke('load_song', { url }),
  getSongHistory: () => invoke('get_song_history'),
  loadHistorySong: id => invoke('load_history_song', { id }),
  removeSongHistory: id => invoke('remove_song_history', { id }),
  clearSongHistory: () => invoke('clear_song_history'),
  loadPreviewAudio: () => invoke('load_preview_audio'),
  downloadSong: options => invoke('download_song', { options }),
  copyLyrics: () => invoke('copy_lyrics'),
  getDownloadFolder: () => invoke('get_download_folder'),
  chooseDownloadFolder: () => invoke('choose_download_folder'),
  resetDownloadFolder: () => invoke('reset_download_folder'),
  revealDownload: () => invoke('reveal_download'),
  onProgress(callback) {
    let unlisten;
    tauri.event.listen('song-progress', event => callback(event.payload)).then(value => {
      unlisten = value;
    });
    return () => unlisten?.();
  },
});
