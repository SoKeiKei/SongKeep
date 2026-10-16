const form = document.getElementById('read-form');
const urlInput = document.getElementById('song-url');
const readButton = document.getElementById('read-button');
const songPanel = document.getElementById('song-panel');
const songTitle = document.getElementById('song-title');
const songDuration = document.getElementById('song-duration');
const lyricsCount = document.getElementById('lyrics-count');
const lyricsText = document.getElementById('lyrics-text');
const copyLyricsButton = document.getElementById('copy-lyrics-button');
const songAuthor = document.getElementById('song-author');
const songPublished = document.getElementById('song-published');
const songPlays = document.getElementById('song-plays');
const songLikes = document.getElementById('song-likes');
const songShares = document.getElementById('song-shares');
const previewButton = document.getElementById('preview-button');
const previewAudio = document.getElementById('preview-audio');
const previewStatus = document.getElementById('preview-status');
const formatInputs = [...document.querySelectorAll('input[name="audio-format"]')];
const saveLyrics = document.getElementById('save-lyrics');
const downloadButton = document.getElementById('download-button');
const downloadState = document.getElementById('download-state');
const progressFill = document.getElementById('progress-fill');
const statusText = document.getElementById('status-text');
const revealButton = document.getElementById('reveal-button');
const downloadFolderPath = document.getElementById('download-folder-path');
const chooseFolderButton = document.getElementById('choose-folder-button');
const resetFolderButton = document.getElementById('reset-folder-button');
const folderStatus = document.getElementById('folder-status');

let busy = false;
let folderBusy = false;
let previewUrl = '';
let loadedUrl = '';
let displayedSong;
let currentLyrics = '';
let copyFeedbackTimer;

function resetPreview() {
  previewAudio.pause();
  previewAudio.removeAttribute('src');
  previewAudio.load();
  previewUrl = '';
  previewButton.textContent = '♪ 试听';
  previewButton.disabled = false;
  previewStatus.textContent = '';
}

previewButton.addEventListener('click', async () => {
  if (!previewUrl) return;
  if (!previewAudio.paused) {
    previewAudio.pause();
    return;
  }
  previewStatus.textContent = '正在加载试听音频…';
  previewButton.disabled = true;
  try {
    if (!previewAudio.getAttribute('src')) {
      const response = await window.sunoApp.loadPreviewAudio();
      if (!response?.ok || !response.dataUrl) throw new Error(response?.error || '试听音频加载失败。');
      previewAudio.src = response.dataUrl;
    }
    await previewAudio.play();
  } catch (_) {
    previewStatus.textContent = '试听失败，请稍后重试。';
    previewButton.textContent = '♪ 重试';
  } finally {
    previewButton.disabled = false;
  }
});

previewAudio.addEventListener('playing', () => {
  previewButton.textContent = 'Ⅱ 暂停';
  previewStatus.textContent = '正在试听';
});
previewAudio.addEventListener('pause', () => {
  if (previewUrl && !previewAudio.ended) {
    previewButton.textContent = '♪ 继续';
    previewStatus.textContent = '已暂停';
  }
});
previewAudio.addEventListener('ended', () => {
  previewButton.textContent = '♪ 重播';
  previewStatus.textContent = '试听结束';
});
previewAudio.addEventListener('error', () => {
  if (previewUrl) previewStatus.textContent = '试听音频加载失败。';
});

function updateButtons() {
  urlInput.disabled = busy;
  readButton.disabled = busy || !urlInput.checkValidity();
  downloadButton.disabled = busy || !urlInput.checkValidity();
  copyLyricsButton.disabled = busy || !currentLyrics;
  chooseFolderButton.disabled = busy || folderBusy;
  resetFolderButton.disabled = busy || folderBusy;
  for (const input of formatInputs) input.disabled = busy;
}

function showCount(value) {
  if (value === null || value === undefined || value === '') return '暂无数据';
  const number = Number(value);
  return Number.isFinite(number) && number >= 0 ? new Intl.NumberFormat('zh-CN').format(number) : '暂无数据';
}

function showPublished(value) {
  if (typeof value !== 'string' || !value.trim()) return '暂无数据';
  const match = value.match(/^\d{4}-\d{2}-\d{2}/);
  if (match) return match[0];
  const date = new Date(value);
  return Number.isNaN(date.getTime()) ? '暂无数据' : date.toLocaleDateString('zh-CN');
}

function showStatus(message, percent = 0, kind = '') {
  downloadState.classList.remove('hidden');
  statusText.textContent = message;
  statusText.className = kind;
  progressFill.style.width = `${Math.max(0, Math.min(100, percent))}%`;
}

function showFolderStatus(message = '', kind = '') {
  folderStatus.textContent = message;
  folderStatus.className = ['folder-status', kind].filter(Boolean).join(' ');
}

function applyFolderSetting(response) {
  const folder = typeof response?.folder === 'string' ? response.folder.trim() : '';
  if (!folder) {
    downloadFolderPath.textContent = '系统默认下载文件夹';
    return;
  }
  downloadFolderPath.textContent = response?.isCustom ? folder : `${folder}（系统默认）`;
}

async function loadFolderSetting() {
  if (typeof window.sunoApp.getDownloadFolder !== 'function') {
    downloadFolderPath.textContent = '系统默认下载文件夹';
    return;
  }
  try {
    const response = await window.sunoApp.getDownloadFolder();
    if (!response?.ok) throw new Error(response?.error || '默认下载位置读取失败。');
    applyFolderSetting(response);
  } catch (error) {
    downloadFolderPath.textContent = '系统默认下载文件夹';
    showFolderStatus(error.message || '默认下载位置读取失败。', 'error');
  }
}

async function chooseFolder() {
  if (busy || folderBusy || typeof window.sunoApp.chooseDownloadFolder !== 'function') return;
  folderBusy = true;
  showFolderStatus('请选择默认下载位置…');
  updateButtons();
  try {
    const response = await window.sunoApp.chooseDownloadFolder();
    if (response?.canceled) {
      showFolderStatus('');
      return;
    }
    if (!response?.ok) throw new Error(response?.error || '默认下载位置设置失败。');
    applyFolderSetting(response);
    showFolderStatus('默认下载位置已更新。', 'success');
  } catch (error) {
    showFolderStatus(error.message || '默认下载位置设置失败。', 'error');
  } finally {
    folderBusy = false;
    updateButtons();
  }
}

async function resetFolder() {
  if (busy || folderBusy || typeof window.sunoApp.resetDownloadFolder !== 'function') return;
  folderBusy = true;
  showFolderStatus('正在恢复系统默认位置…');
  updateButtons();
  try {
    const response = await window.sunoApp.resetDownloadFolder();
    if (!response?.ok) throw new Error(response?.error || '恢复默认下载位置失败。');
    applyFolderSetting(response);
    showFolderStatus('已恢复系统默认下载位置。', 'success');
  } catch (error) {
    showFolderStatus(error.message || '恢复默认下载位置失败。', 'error');
  } finally {
    folderBusy = false;
    updateButtons();
  }
}

function showDuration(seconds) {
  if (!Number.isFinite(seconds) || seconds <= 0) {
    songDuration.textContent = '';
    return;
  }
  const minutes = Math.floor(seconds / 60);
  const remaining = Math.floor(seconds % 60);
  songDuration.textContent = `${minutes}:${String(remaining).padStart(2, '0')}`;
}

function applySong(song, url, preserveLyricsChoice = false) {
  resetPreview();
  clearTimeout(copyFeedbackTimer);
  copyLyricsButton.textContent = '复制歌词';
  displayedSong = song;
  loadedUrl = url;
  songTitle.textContent = song.title;
  previewUrl = song.previewUrl || '';
  previewButton.disabled = !previewUrl;
  showDuration(Number(song.duration));
  songAuthor.textContent = song.author || '暂无数据';
  songPublished.textContent = showPublished(song.publishedAt);
  songPlays.textContent = showCount(song.playCount);
  songLikes.textContent = showCount(song.likeCount);
  songShares.textContent = showCount(song.shareCount);
  currentLyrics = typeof song.lyrics === 'string' ? song.lyrics : '';
  lyricsText.textContent = currentLyrics || '这首歌曲没有公开歌词。';
  lyricsCount.textContent = currentLyrics ? `${currentLyrics.split(/\r?\n/).length} 行` : '暂无歌词';
  saveLyrics.disabled = !currentLyrics;
  if (!preserveLyricsChoice || !currentLyrics) saveLyrics.checked = false;
  songPanel.classList.remove('hidden');
  updateButtons();
}

urlInput.addEventListener('input', () => {
  const url = urlInput.value.trim();
  if (url !== loadedUrl) {
    songPanel.classList.add('hidden');
    resetPreview();
    currentLyrics = '';
    revealButton.classList.add('hidden');
    if (url) showStatus('当前链接还未读取，可先读取或直接下载。');
    else downloadState.classList.add('hidden');
  } else if (displayedSong && songPanel.classList.contains('hidden')) {
    applySong(displayedSong, loadedUrl, true);
    showStatus('歌曲信息已读取，可以下载到本地。', 0, 'success');
  }
  updateButtons();
});

form.addEventListener('submit', async event => {
  event.preventDefault();
  const url = urlInput.value.trim();
  if (!url || busy) return;
  busy = true;
  resetPreview();
  currentLyrics = '';
  songPanel.classList.add('hidden');
  revealButton.classList.add('hidden');
  readButton.textContent = '读取中…';
  showStatus('正在读取歌曲信息…');
  updateButtons();
  try {
    const response = await window.sunoApp.loadSong(url);
    if (!response.ok) throw new Error(response.error);
    applySong(response.song, url);
    showStatus('歌曲信息已读取，点击“下载到本地”即可保存。', 0, 'success');
  } catch (error) {
    showStatus(error.message || '读取失败，请检查链接后重试。', 0, 'error');
  } finally {
    busy = false;
    readButton.textContent = '读取歌曲';
    updateButtons();
  }
});

chooseFolderButton.addEventListener('click', chooseFolder);
resetFolderButton.addEventListener('click', resetFolder);

copyLyricsButton.addEventListener('click', async () => {
  if (!currentLyrics || busy) return;
  try {
    const response = await window.sunoApp.copyLyrics();
    if (!response.ok) throw new Error(response.error);
    copyLyricsButton.textContent = '已复制';
    clearTimeout(copyFeedbackTimer);
    copyFeedbackTimer = setTimeout(() => { copyLyricsButton.textContent = '复制歌词'; }, 2000);
  } catch (error) {
    showStatus(error.message || '复制歌词失败。', 0, 'error');
  }
});

window.sunoApp.onProgress(progress => {
  const percent = typeof progress === 'number' ? progress : Number(progress?.percent || 0);
  const message = typeof progress === 'object' && progress?.message
    ? progress.message
    : `正在下载… ${Math.round(percent)}%`;
  showStatus(message, percent);
});

downloadButton.addEventListener('click', async () => {
  if (busy || !urlInput.checkValidity()) return;
  const url = urlInput.value.trim();
  busy = true;
  revealButton.classList.add('hidden');
  downloadButton.textContent = '下载中…';
  showStatus('正在准备下载…', 0);
  updateButtons();
  try {
    const format = formatInputs.find(input => input.checked)?.value || 'mp3';
    const response = await window.sunoApp.downloadSong({ url, format, saveLyrics: saveLyrics.checked });
    if (response.song && (loadedUrl !== url || songPanel.classList.contains('hidden'))) {
      applySong(response.song, url, true);
    }
    if (response.canceled) {
      showStatus('已取消。', 0);
      return;
    }
    if (!response.ok) throw new Error(response.error);
    showStatus('下载完成，已保存到本地。', 100, 'success');
    revealButton.classList.remove('hidden');
  } catch (error) {
    showStatus(error.message || '下载失败，请稍后重试。', 0, 'error');
  } finally {
    busy = false;
    downloadButton.textContent = '下载到本地';
    updateButtons();
  }
});

revealButton.addEventListener('click', () => window.sunoApp.revealDownload());
updateButtons();
loadFolderSetting();
