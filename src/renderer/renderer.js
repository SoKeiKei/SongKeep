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
const previewControls = document.getElementById('preview-controls');
const previewSeek = document.getElementById('preview-seek');
const previewCurrentTime = document.getElementById('preview-current-time');
const previewTotalTime = document.getElementById('preview-total-time');
const historyPanel = document.getElementById('history-panel');
const historyCount = document.getElementById('history-count');
const historyStatus = document.getElementById('history-status');
const historyEmpty = document.getElementById('history-empty');
const historyList = document.getElementById('history-list');
const clearHistoryButton = document.getElementById('clear-history-button');
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
const themeSelect = document.getElementById('theme-select');

const themes = new Set(['forest', 'sea', 'sunset', 'lavender', 'night']);
const savedTheme = localStorage.getItem('songkeep-theme');
const activeTheme = themes.has(savedTheme) ? savedTheme : 'forest';
document.documentElement.dataset.theme = activeTheme;
themeSelect.value = activeTheme;

themeSelect.addEventListener('change', () => {
  const theme = themes.has(themeSelect.value) ? themeSelect.value : 'forest';
  document.documentElement.dataset.theme = theme;
  localStorage.setItem('songkeep-theme', theme);
});

let busy = false;
let folderBusy = false;
let previewUrl = '';
let loadedUrl = '';
let displayedSong;
let currentLyrics = '';
let copyFeedbackTimer;
let songHistory = [];

function formatTime(seconds) {
  if (!Number.isFinite(seconds) || seconds < 0) return '0:00';
  const wholeSeconds = Math.floor(seconds);
  const hours = Math.floor(wholeSeconds / 3600);
  const minutes = Math.floor((wholeSeconds % 3600) / 60);
  const remaining = wholeSeconds % 60;
  return hours > 0
    ? `${hours}:${String(minutes).padStart(2, '0')}:${String(remaining).padStart(2, '0')}`
    : `${minutes}:${String(remaining).padStart(2, '0')}`;
}

function updatePreviewTimeline() {
  const duration = Number(previewAudio.duration);
  const canSeek = Boolean(previewUrl) && Number.isFinite(duration) && duration > 0;
  previewSeek.disabled = !canSeek;
  previewSeek.max = canSeek ? String(duration) : '0';
  if (!previewSeek.matches(':active')) previewSeek.value = canSeek ? String(previewAudio.currentTime || 0) : '0';
  previewCurrentTime.textContent = formatTime(previewAudio.currentTime || 0);
  previewTotalTime.textContent = canSeek ? formatTime(duration) : formatTime(Number(displayedSong?.duration));
}

function resetPreview() {
  previewAudio.pause();
  previewAudio.removeAttribute('src');
  previewAudio.load();
  previewUrl = '';
  previewControls.classList.add('hidden');
  previewSeek.disabled = true;
  previewSeek.max = '0';
  previewSeek.value = '0';
  previewCurrentTime.textContent = '0:00';
  previewTotalTime.textContent = '0:00';
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
previewAudio.addEventListener('loadedmetadata', updatePreviewTimeline);
previewAudio.addEventListener('durationchange', updatePreviewTimeline);
previewAudio.addEventListener('timeupdate', updatePreviewTimeline);
previewAudio.addEventListener('seeked', updatePreviewTimeline);

previewSeek.addEventListener('input', () => {
  if (previewSeek.disabled) return;
  previewAudio.currentTime = Number(previewSeek.value);
  previewCurrentTime.textContent = formatTime(previewAudio.currentTime);
});

function setHistoryStatus(message = '', kind = '') {
  historyStatus.textContent = message;
  historyStatus.className = ['history-status', kind].filter(Boolean).join(' ');
}

function renderSongHistory(songs) {
  songHistory = Array.isArray(songs) ? songs : [];
  historyCount.textContent = String(songHistory.length);
  historyEmpty.classList.toggle('hidden', songHistory.length > 0);
  clearHistoryButton.disabled = busy || songHistory.length === 0;
  historyList.replaceChildren(...songHistory.map(song => {
    const item = document.createElement('li');
    item.className = 'history-item';

    const info = document.createElement('div');
    info.className = 'history-item-info';
    const title = document.createElement('strong');
    title.className = 'history-item-title';
    title.textContent = song.title || '未命名歌曲';
    const detail = document.createElement('span');
    detail.className = 'history-item-detail';
    const duration = Number(song.duration);
    const metadata = [
      song.author,
      Number.isFinite(duration) && duration > 0 ? formatTime(duration) : '',
    ].filter(Boolean);
    detail.textContent = metadata.join(' · ') || song.sourceUrl || song.id;
    info.append(title, detail);

    const actions = document.createElement('div');
    actions.className = 'history-item-actions';
    for (const [action, label, className] of [
      ['open', '打开', 'history-open'],
      ['download', '下载', 'history-download'],
      ['remove', '删除', 'history-remove'],
    ]) {
      const button = document.createElement('button');
      button.type = 'button';
      button.className = className;
      button.dataset.action = action;
      button.dataset.id = song.id;
      button.textContent = label;
      button.disabled = busy;
      actions.append(button);
    }
    item.append(info, actions);
    return item;
  }));
}

async function refreshSongHistory() {
  if (typeof window.sunoApp.getSongHistory !== 'function') return;
  try {
    const response = await window.sunoApp.getSongHistory();
    if (!response?.ok) throw new Error(response?.error || '读取历史记录失败。');
    renderSongHistory(response.songs);
    setHistoryStatus('');
  } catch (error) {
    clearHistoryButton.disabled = busy;
    setHistoryStatus(error.message || '读取历史记录失败。', 'error');
  }
}

async function loadHistoryEntry(id) {
  if (busy || typeof window.sunoApp.loadHistorySong !== 'function') return '';
  busy = true;
  updateButtons();
  setHistoryStatus('正在载入历史歌曲…');
  try {
    const response = await window.sunoApp.loadHistorySong(id);
    if (!response?.ok || !response.song) throw new Error(response?.error || '载入历史歌曲失败。');
    const url = response.song.sourceUrl;
    urlInput.value = url;
    applySong(response.song, url);
    showStatus('已从历史记录载入歌曲，可以直接试听或下载。', 0, 'success');
    historyPanel.open = true;
    await refreshSongHistory();
    setHistoryStatus(response.historyWarning || '', response.historyWarning ? 'warning' : '');
    return url;
  } catch (error) {
    setHistoryStatus(error.message || '载入历史歌曲失败。', 'error');
    return '';
  } finally {
    busy = false;
    updateButtons();
  }
}

async function removeHistoryEntry(id) {
  if (busy || typeof window.sunoApp.removeSongHistory !== 'function') return;
  busy = true;
  updateButtons();
  setHistoryStatus('正在删除历史记录…');
  try {
    const response = await window.sunoApp.removeSongHistory(id);
    if (!response?.ok) throw new Error(response?.error || '删除历史记录失败。');
    await refreshSongHistory();
    setHistoryStatus('已从历史记录删除。', 'success');
  } catch (error) {
    setHistoryStatus(error.message || '删除历史记录失败。', 'error');
  } finally {
    busy = false;
    updateButtons();
  }
}

async function clearSongHistory() {
  if (busy || typeof window.sunoApp.clearSongHistory !== 'function') return;
  if (!window.confirm('确定清空本机保存的全部历史记录吗？此操作无法撤销。')) return;
  busy = true;
  updateButtons();
  setHistoryStatus('正在清空历史记录…');
  try {
    const response = await window.sunoApp.clearSongHistory();
    if (!response?.ok) throw new Error(response?.error || '清空历史记录失败。');
    renderSongHistory([]);
    setHistoryStatus('历史记录已清空。', 'success');
  } catch (error) {
    setHistoryStatus(error.message || '清空历史记录失败。', 'error');
  } finally {
    busy = false;
    updateButtons();
  }
}

function updateButtons() {
  urlInput.disabled = busy;
  readButton.disabled = busy || !urlInput.checkValidity();
  downloadButton.disabled = busy || !urlInput.checkValidity();
  copyLyricsButton.disabled = busy || !currentLyrics;
  chooseFolderButton.disabled = busy || folderBusy;
  resetFolderButton.disabled = busy || folderBusy;
  clearHistoryButton.disabled = busy || songHistory.length === 0;
  for (const button of historyList.querySelectorAll('button')) button.disabled = busy;
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
  previewControls.classList.toggle('hidden', !previewUrl);
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
  updatePreviewTimeline();
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
    await refreshSongHistory();
    showStatus(
      response.historyWarning
        ? `歌曲已读取，但${response.historyWarning}`
        : '歌曲信息已读取，点击“下载到本地”即可保存。',
      0,
      response.historyWarning ? 'warning' : 'success',
    );
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
clearHistoryButton.addEventListener('click', clearSongHistory);

historyList.addEventListener('click', async event => {
  const button = event.target.closest('button[data-action][data-id]');
  if (!button || busy) return;
  const { action, id } = button.dataset;
  if (action === 'remove') {
    await removeHistoryEntry(id);
    return;
  }
  const url = await loadHistoryEntry(id);
  if (url && action === 'download') await runDownload(url);
});

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

async function runDownload(url) {
  if (busy || !url) return;
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
    if (response.song) await refreshSongHistory();
    if (response.canceled) {
      showStatus(response.historyWarning || '已取消。', 0, response.historyWarning ? 'warning' : '');
      return;
    }
    if (!response.ok) throw new Error(response.error);
    showStatus(
      response.historyWarning
        ? `下载完成，已保存到本地，但${response.historyWarning}`
        : '下载完成，已保存到本地。',
      100,
      response.historyWarning ? 'warning' : 'success',
    );
    revealButton.classList.remove('hidden');
  } catch (error) {
    showStatus(error.message || '下载失败，请稍后重试。', 0, 'error');
  } finally {
    busy = false;
    downloadButton.textContent = '下载到本地';
    updateButtons();
  }
}

downloadButton.addEventListener('click', async () => {
  if (!urlInput.checkValidity()) return;
  await runDownload(urlInput.value.trim());
});

revealButton.addEventListener('click', () => window.sunoApp.revealDownload());
updateButtons();
loadFolderSetting();
refreshSongHistory();
