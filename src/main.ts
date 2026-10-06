import { invoke, convertFileSrc, isTauri } from '@tauri-apps/api/core';
import { ArchiveAudio } from './audio';
import { ImagesView, selectedText } from './images';
import type { Device, ImportReport, Recording, Run, Settings, Snapshot, Transcript } from './types';
import './style.css';

const app = document.querySelector<HTMLDivElement>('#app')!;
const icons = {
  wave: '<svg viewBox="0 0 32 32" fill="none" aria-hidden="true"><path d="M5 13v6m5-10v14m6-19v24m6-19v14m5-10v6" stroke="currentColor" stroke-width="3" stroke-linecap="round"/></svg>',
  library: '<svg viewBox="0 0 24 24" fill="none" aria-hidden="true"><rect x="4" y="4" width="16" height="16" rx="3" stroke="currentColor" stroke-width="1.6"/><path d="M4 10h16M10 10v10" stroke="currentColor" stroke-width="1.6"/></svg>',
  device: '<svg viewBox="0 0 24 24" fill="none" aria-hidden="true"><rect x="7" y="3" width="10" height="18" rx="3" stroke="currentColor" stroke-width="1.6"/><path d="M10 7h4m-2 7v3" stroke="currentColor" stroke-width="1.6"/></svg>',
  settings: '<svg viewBox="0 0 24 24" fill="none" aria-hidden="true"><path d="M4 7h16M4 17h16" stroke="currentColor" stroke-width="1.6"/><circle cx="9" cy="7" r="3" fill="currentColor"/><circle cx="16" cy="17" r="3" fill="currentColor"/></svg>',
  plus: '<svg viewBox="0 0 24 24" fill="none" aria-hidden="true"><path d="M12 5v14M5 12h14" stroke="currentColor" stroke-width="1.8" stroke-linecap="round"/></svg>',
};
let state: Snapshot | null = null;
let selected: string | null = null;
let activeRun: string | null = null;
let loaded: Transcript | null = null;
let audioSource: ArchiveAudio | null = null;
let selectionEvents: AbortController | null = null;
let page: 'library' | 'queue' | 'images' | 'devices' | 'settings' = 'library';
let filter = '';
let loadSequence = 0;
let dirty = false;
let refreshing = false;
let busy = false;
let toastTimer: ReturnType<typeof setTimeout>;
const labels: Record<string, string> = { queued: 'I kön', running: 'Transkriberar', completed: 'Klar', failed: 'Misslyckad' };
const esc = (s: unknown) => String(s ?? '').replace(/[&<>"']/g, c => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' })[c]!);
const clock = (s: number) => `${Math.floor(s / 60).toString().padStart(2, '0')}:${Math.floor(s % 60).toString().padStart(2, '0')}`;
const date = (s: string | null) => s ? new Date(s).toLocaleString('sv-SE', { dateStyle: 'medium', timeStyle: 'short' }) : 'Datum okänt';
const $ = <T extends HTMLElement = HTMLElement>(selector: string) => document.querySelector<T>(selector)!;

app.innerHTML = `
  <aside class="sidebar">
    <div class="brand"><span class="brand-icon">${icons.wave}</span><div>DreamWhisper<small>DINA ORD, BEVARADE.</small></div></div>
    <div class="nav-label">DITT ARKIV</div>
    <nav aria-label="Huvudmeny">
      <button data-page="library" class="nav active">${icons.library}Inspelningar<span id="count">0</span></button>
      <button data-page="queue" class="nav">${icons.wave}Arbetskö<span id="queue-count">0</span></button>
      <button data-page="images" class="nav">${icons.plus}Bilder</button>
      <button data-page="devices" class="nav">${icons.device}Diktafon</button>
      <button data-page="settings" class="nav">${icons.settings}Inställningar</button>
    </nav>
    <div class="sidebar-bottom"><span class="privacy-dot"></span> Allt stannar på din dator<small>Svenskt tal. Lokal transkribering.</small></div>
  </aside>
  <main>
    <header><div class="breadcrumb">PERSONLIGT LJUDARKIV <span>/</span> <b id="page-label">Inspelningar</b></div><span class="local-badge">● Lokalt</span></header>
    <div id="notice" class="notice hidden" role="status"></div>
    <section id="library-page">
      <div class="page-heading"><div><div class="eyebrow">FRÅN RÖST TILL TEXT</div><h1>Dina inspelningar</h1><p>Fånga tanken. Hitta orden igen.</p></div><button id="import-open" class="primary">${icons.plus}Importera ljud</button></div>
      <div class="overview"><div><span>ARKIVERADE</span><strong id="stat-all">0</strong><small>inspelningar</small></div><div><span>TRANSKRIBERADE</span><strong id="stat-done">0</strong><small>redo att läsa</small></div><div><span>I ARBETSKÖN</span><strong id="stat-queue">0</strong><small>väntar på transkribering</small></div><div class="engine"><span class="engine-dot"></span><div><b>KB-Whisper large</b><small>SVENSKA · FP16 · CUDA</small><p id="engine-status">Transkribering pausad</p></div></div></div>
      <div class="workspace"><section class="recording-panel"><div class="list-head"><h2>Bibliotek</h2><span id="list-count">0 filer</span></div><label class="search-label"><span aria-hidden="true">⌕</span><input id="search" type="search" placeholder="Sök namn eller transkript…" aria-label="Sök inspelningar" /></label><div id="recording-list"></div></section>
      <section class="detail-panel" id="detail"><div class="detail-empty"><span class="empty-icon">${icons.wave}</span><h2>En tanke börjar med din röst</h2><p>Anslut din Sony-diktafon eller importera en mapp med inspelningar. Välj sedan en fil för att lyssna och läsa.</p><span class="format-note">MP3 & WAV · SVENSK TRANSKRIBERING</span></div></section></div>
    </section>
    <section id="images-page" class="hidden"></section><section id="queue-page" class="hidden"></section><section id="devices-page" class="hidden"></section><section id="settings-page" class="hidden"></section>
    <footer><span id="activity">Redo</span><span id="archive-location"></span></footer>
  </main>
  <dialog id="import-dialog"><form id="import-form"><div class="eyebrow">LOKAL IMPORT</div><h2>Importera inspelningar</h2><p>Ange mappen med ljudfiler. Undermappar tas med och originalen bevaras.</p><label>Mappens fullständiga sökväg<input id="import-path" required placeholder="/run/media/ditt-namn/IC RECORDER/PRIVATE/SONY/REC_FILE" /></label><div class="dialog-actions"><button type="button" id="import-cancel" class="secondary">Avbryt</button><button type="submit" class="primary">Importera mapp</button></div></form></dialog>
  <div id="toast" role="status" class="toast hidden"></div>`;

const imagesView = new ImagesView($('#images-page'), () => document.querySelector('#comfy-settings'), toast, (id, runId) => {
  if (dirty && !window.confirm('Du har osparade rättningar. Lämna dem?')) return;
  dirty = false;
  selected = id; navigate('library'); void action(() => loadDetail(id, runId ?? undefined));
});

function toast(message: string, error = false) {
  clearTimeout(toastTimer);
  const element = $('#toast'); element.textContent = message;
  element.className = `toast${error ? ' error' : ''}`;
  toastTimer = setTimeout(() => element.classList.add('hidden'), error ? 15000 : 6500);
}
async function action<T>(callback: () => Promise<T>): Promise<T | undefined> {
  try { return await callback(); } catch (error) { toast(String(error), true); }
}
function importMessage(report: ImportReport) {
  toast(`${report.imported} importerade · ${report.skipped} redan i arkivet${report.errors.length ? ` · ${report.errors.length} fel: ${report.errors.join('; ')}` : ''}`, report.errors.length > 0);
}
async function refresh() {
  if (!isTauri() || refreshing) return;
  refreshing = true;
  try {
    const previous = state;
    const [snapshot] = await Promise.all([invoke<Snapshot>('snapshot'), imagesView.refresh()]);
    state = snapshot;
    $('#count').textContent = String(state.recordings.length);
    $('#stat-all').textContent = String(state.recordings.length);
    $('#stat-done').textContent = String(state.recordings.filter(r => r.status === 'completed').length);
    $('#stat-queue').textContent = String(state.recordings.filter(r => ['queued', 'running'].includes(r.status)).length);
    $('#queue-count').textContent = String(state.queue.length);
    $('#activity').textContent = state.activity;
    $('#engine-status').textContent = queueStatus();
    $('#archive-location').textContent = state.data_dir;
    $('#archive-location').title = state.data_dir;
    const notice = $('#notice');
    notice.textContent = state.device_error ?? '';
    notice.classList.toggle('hidden', !state.device_error);
    if (page === 'library') {
      renderList();
      const oldStatus = previous?.recordings.find(r => r.id === selected)?.status;
      const newStatus = state.recordings.find(r => r.id === selected)?.status;
      if (selected && oldStatus !== newStatus && !dirty) await loadDetail(selected);
    }
    if (page === 'queue') renderQueue();
    if (page === 'devices') renderDevices();
  } catch (error) { $('#activity').textContent = String(error); }
  finally { refreshing = false; }
}
let searchResults: Set<string> | null = null;
function renderList() {
  if (!state) return;
  const recordings = state.recordings.filter(r => !filter || r.name.toLocaleLowerCase('sv').includes(filter) || searchResults?.has(r.id));
  $('#list-count').textContent = `${recordings.length} filer`;
  $('#recording-list').innerHTML = recordings.length ? recordings.map(r => `
    <button class="recording-row ${selected === r.id ? 'selected' : ''}" data-recording="${esc(r.id)}">
      <span class="file-icon">${icons.wave}</span><span class="row-info"><b>${esc(r.name)}</b><small>${esc(date(r.source_modified_at))}</small><span class="row-meta"><span class="status ${esc(r.status)}">${esc(labels[r.status])}</span><span>${r.duration ? clock(r.duration) : `${(r.size / 1024 / 1024).toFixed(1)} MB`}</span></span></span><span class="row-arrow">›</span>
    </button>`).join('') : `<div class="list-empty">${filter ? 'Inga inspelningar matchar din sökning.' : 'Här samlas dina inspelningar.<br>Importera ljud för att börja.'}</div>`;
  document.querySelectorAll<HTMLButtonElement>('[data-recording]').forEach(button => button.onclick = () => {
    if (dirty && !window.confirm('Du har osparade rättningar. Lämna dem?')) return;
    void action(() => loadDetail(button.dataset.recording!));
  });
}
async function loadDetail(id: string, runId?: string) {
  const sequence = ++loadSequence;
  const recording = state?.recordings.find(r => r.id === id);
  if (!recording) return;
  const [transcript, runs] = await Promise.all([
    invoke<Transcript>('transcript', { id, runId: runId ?? null }), invoke<Run[]>('transcription_runs', { id }),
  ]);
  if (sequence !== loadSequence || page !== 'library') return;
  selected = id; activeRun = transcript.run_id; loaded = transcript; dirty = false;
  renderList();
  renderDetail(recording, transcript, runs);
}
function renderDetail(recording: Recording, transcript: Transcript, runs: Run[]) {
  audioSource?.dispose();
  audioSource = null;
  selectionEvents?.abort();
  $('#detail').innerHTML = `
    <div class="detail-head"><div class="eyebrow">INSPELNING</div><h2>${esc(recording.name)}</h2><div class="detail-meta">${esc(date(recording.source_modified_at))}<span>·</span>${(recording.size / 1024 / 1024).toFixed(1)} MB<span>·</span><span class="status ${esc(recording.status)}">${esc(labels[recording.status])}</span></div></div>
    <div class="player"><span class="player-label">ORIGINALLJUD</span><audio id="audio" controls preload="metadata"></audio><div id="audio-error" class="hidden inline-error"></div></div>
    ${recording.error ? `<p class="inline-error">${esc(recording.error)}</p>` : ''}
    <div class="transcript-tools"><h3>Transkript <span>SV</span></h3><div class="tool-actions"><button id="retry" class="text-button" ${['running', 'queued'].includes(recording.status) ? 'disabled' : ''}>${recording.status === 'queued' ? 'Redan i kön' : recording.status === 'running' ? 'Transkriberar' : transcript.run_id ? 'Ny transkribering' : 'Försök igen'}</button>${transcript.run_id ? '<select id="export-format" aria-label="Exportformat"><option value="txt">TXT</option><option value="md">Markdown</option><option value="srt">SRT</option></select><button id="export" class="secondary small">Exportera ↗</button>' : ''}</div></div>
    ${runs.length > 1 ? `<label class="version-label">Version<select id="version">${runs.map((run, i) => `<option value="${esc(run.id)}" ${run.id === transcript.run_id ? 'selected' : ''}>${esc(date(run.created_at))}${i === 0 ? ' · senaste' : ''}</option>`).join('')}</select></label>` : ''}
    ${transcript.segments.length ? '<details class="image-selection"><summary>Markera text för en bild</summary><p class="muted">Markera valfri text, även över flera stycken. Bildprompten kan ändras innan du skapar bilden.</p><div id="image-transcript-text" class="selectable-transcript" tabindex="0"></div><button id="image-from-selection" class="secondary" disabled>Skapa bild av markering</button></details>' : ''}
    <div class="transcript-content">${transcript.run_id ? (transcript.segments.length ? transcript.segments.map(s => `<article class="segment" data-start="${s.start}" data-end="${s.end}"><button class="timestamp" data-seek="${s.start}" title="Spela från denna tid">${clock(s.start)}</button><textarea data-segment="${s.id}" aria-label="Text vid ${clock(s.start)}" rows="2">${esc(s.edited_text ?? s.text)}</textarea></article>`).join('') : '<p class="transcript-empty">Inget tal hittades i inspelningen.</p>') : `<div class="transcript-empty"><p>${recording.status === 'running' ? 'Din inspelning transkriberas …' : recording.status === 'failed' ? 'Transkriberingen misslyckades. Åtgärda felet och försök igen.' : 'Inspelningen ligger i arbetskön.'}</p><small>${state?.settings.transcription_enabled ? 'Texten visas när transkriberingen är klar.' : 'Kön är pausad. Öppna Arbetskö och välj Starta kön.'}</small></div>`}</div>
    ${transcript.run_id ? '<div class="save-bar"><span id="edit-status">Tidsstämplarna följer originalljudet.</span><button id="save-edits" class="primary small" disabled>Spara rättningar</button></div>' : ''}`;
  const audio = $<HTMLAudioElement>('#audio');
  const audioError = $('#audio-error');
  const showAudioError = (message: string) => { audioError.textContent = message; audioError.classList.remove('hidden'); };
  audio.addEventListener('error', () => {
    const messages: Record<number, string> = {
      1: 'Ljudinläsningen avbröts.',
      2: 'Ljudfilen kunde inte läsas.',
      3: 'Ljudfilen kunde inte avkodas. Filen kan vara skadad.',
      4: 'Ljudformatet stöds inte av datorns ljudspelare.',
    };
    showAudioError(messages[audio.error?.code ?? 0] ?? 'Ljudet kunde inte spelas.');
  });
  const source = new ArchiveAudio(audio, convertFileSrc(recording.archive_path), recording.archive_path.toLowerCase().endsWith('.mp3') ? 'audio/mpeg' : 'audio/wav');
  audioSource = source;
  void source.ready.catch(error => {
    if (audioSource === source) showAudioError(`Ljudet kunde inte laddas: ${String(error)}`);
  });
  audio.addEventListener('timeupdate', () => {
    if (audioSource !== source) return;
    const time = audio.currentTime;
    document.querySelectorAll<HTMLElement>('.segment').forEach(el => el.classList.toggle('playing', time >= Number(el.dataset.start) && time < Number(el.dataset.end)));
  });
  document.querySelectorAll<HTMLButtonElement>('[data-seek]').forEach(button => button.onclick = () => {
    void action(() => source.playFrom(Number(button.dataset.seek)));
  });
  document.querySelectorAll<HTMLTextAreaElement>('[data-segment]').forEach(input => {
    input.oninput = () => { dirty = true; $<HTMLButtonElement>('#save-edits').disabled = false; $('#edit-status').textContent = 'Osparade rättningar'; };
    input.style.height = `${Math.max(input.scrollHeight, 58)}px`;
  });
  const selectionText = document.querySelector<HTMLElement>('#image-transcript-text');
  if (selectionText) {
    const selectionButton = $<HTMLButtonElement>('#image-from-selection');
    const updateSelectionText = () => {
      selectionText.textContent = Array.from(document.querySelectorAll<HTMLTextAreaElement>('[data-segment]')).map(input => input.value).join('\n\n');
      selectionButton.disabled = true;
    };
    updateSelectionText();
    let captured = '';
    selectionEvents = new AbortController();
    document.addEventListener('selectionchange', () => {
      captured = selectedText(selectionText, window.getSelection());
      selectionButton.disabled = !captured;
    }, { signal: selectionEvents.signal });
    document.querySelectorAll<HTMLTextAreaElement>('[data-segment]').forEach(input => input.addEventListener('input', updateSelectionText));
    selectionButton.onmousedown = event => event.preventDefault();
    selectionButton.onclick = () => void action(async () => {
      const text = captured;
      if (!text) return;
      if (dirty && !window.confirm('Bildprompten använder dina aktuella rättningar. Lämna osparade transkripträttningar?')) return;
      if (await imagesView.fromSelection(text, recording.id, transcript.run_id)) { dirty = false; navigate('images'); }
    });
  }
  $('#retry').onclick = () => void action(async () => { await invoke('retry_recording', { id: recording.id }); toast(`Inspelningen har lagts i kön.${transcript.run_id ? ' Tidigare transkript finns kvar.' : ''}${state?.settings.transcription_enabled ? '' : ' Kön är pausad – välj Starta kön.'}`); await refresh(); navigate('queue'); });
  document.querySelector<HTMLSelectElement>('#version')?.addEventListener('change', event => {
    if (dirty && !window.confirm('Du har osparade rättningar. Byta version?')) { (event.target as HTMLSelectElement).value = activeRun!; return; }
    void action(() => loadDetail(recording.id, (event.target as HTMLSelectElement).value));
  });
  document.querySelector('#save-edits')?.addEventListener('click', () => void action(saveEdits));
  document.querySelector('#export')?.addEventListener('click', () => void action(async () => {
    if (dirty) await saveEdits();
    const path = await invoke<string>('export_transcript', { id: recording.id, runId: activeRun, format: $<HTMLSelectElement>('#export-format').value });
    toast(`Export sparad: ${path}`);
  }));
}
async function saveEdits() {
  if (!loaded) return;
  const button = $<HTMLButtonElement>('#save-edits'); button.disabled = true;
  try {
    for (const input of document.querySelectorAll<HTMLTextAreaElement>('[data-segment]')) {
      const segment = loaded.segments.find(s => s.id === Number(input.dataset.segment));
      if (segment && input.value !== (segment.edited_text ?? segment.text)) {
        await invoke('save_segment', { id: segment.id, text: input.value }); segment.edited_text = input.value;
      }
    }
    dirty = false; $('#edit-status').textContent = 'Rättningar sparade · originalljud och maskintext bevarade';
    toast('Rättningar sparade');
  } catch (error) { button.disabled = false; throw error; }
}
function queueStatus() {
  if (!state) return 'Hämtar köstatus …';
  if (!state.settings.transcription_enabled) return state.worker_status === 'running' ? 'Pausar efter pågående inspelning' : 'Transkribering pausad';
  return ({ loading: 'Laddar modellen …', running: 'Transkriberar', error: 'Transkriberingsmotorn behöver åtgärdas', ready: 'Redo för transkribering', awaiting_images: 'Väntar på bildskapande' } as Record<string, string>)[state.worker_status] ?? 'Startar transkribering …';
}
function renderQueue() {
  if (!state) return;
  const snapshot = state;
  const pending = snapshot.queue.map(id => snapshot.recordings.find(r => r.id === id)).filter((r): r is Recording => !!r);
  const failed = snapshot.recordings.filter(r => r.status === 'failed');
  let position = 0;
  const row = (r: Recording, retry = false) => `<article class="queue-row"><span class="queue-position">${retry ? '!' : r.status === 'running' ? '▶' : ++position}</span><div class="queue-info"><b>${esc(r.name)}</b><small>${esc(labels[r.status])} · ${r.attempts} försök</small>${r.error ? `<p class="inline-error">${esc(r.error)}</p>` : ''}</div><button class="secondary small" data-open-queue="${esc(r.id)}">Öppna</button>${retry ? `<button class="secondary small" data-queue-retry="${esc(r.id)}">Försök igen</button>` : ''}</article>`;
  $('#queue-page').innerHTML = `<div class="page-heading"><div><div class="eyebrow">FRÅN LJUD TILL TEXT</div><h1>Arbetskö</h1><p>${pending.length} pågående eller väntande · ${failed.length} misslyckade</p></div><div class="tool-actions"><button id="queue-control" class="primary">${snapshot.settings.transcription_enabled && snapshot.worker_status !== 'error' ? 'Pausa kön' : snapshot.worker_status === 'error' ? 'Starta om motorn' : 'Starta kön'}</button><button id="queue-settings" class="secondary">Inställningar</button></div></div>
    <div class="settings-card queue-summary" role="status"><h2>${esc(queueStatus())}</h2><p>${!snapshot.settings.transcription_enabled ? 'Väntande filer är sparade. Starta kön för att transkribera dem. En pågående inspelning slutförs innan kön pausas.' : 'Filer transkriberas en i taget i ordningen nedan. Kön sparas även när appen stängs.'}</p>${snapshot.worker_error ? `<p class="inline-error">${esc(snapshot.worker_error)}</p><p>Kontrollera Python och modellmappen under Inställningar och starta sedan om motorn.</p>` : ''}${snapshot.worker_status === 'loading' || snapshot.worker_status === 'running' ? `<p>${esc(snapshot.activity)}</p>` : ''}</div>
    <div class="settings-card queue-list"><h2>Pågående och väntande</h2>${pending.length ? pending.map(r => row(r)).join('') : '<p class="muted">Inga inspelningar väntar. Importera ljud eller välj Ny transkribering i biblioteket.</p>'}</div>
    ${failed.length ? `<div class="settings-card queue-list"><h2>Misslyckade</h2>${failed.map(r => row(r, true)).join('')}</div>` : ''}`;
  $('#queue-control').onclick = () => void action(async () => {
    const enabled = !snapshot.settings.transcription_enabled || snapshot.worker_status === 'error';
    $<HTMLButtonElement>('#queue-control').disabled = true;
    try { await invoke('control_queue', { enabled }); await refresh(); }
    finally { if (page === 'queue') renderQueue(); }
  });
  $('#queue-settings').onclick = () => navigate('settings');
  document.querySelectorAll<HTMLButtonElement>('[data-open-queue]').forEach(b => b.onclick = () => {
    selected = b.dataset.openQueue!; navigate('library');
  });
  document.querySelectorAll<HTMLButtonElement>('[data-queue-retry]').forEach(b => b.onclick = () => void action(async () => {
    await invoke('retry_recording', { id: b.dataset.queueRetry });
    toast(snapshot.settings.transcription_enabled ? 'Inspelningen har lagts i kön.' : 'Inspelningen har lagts i kön. Välj Starta kön för att fortsätta.');
    await refresh();
  }));
}
function renderDevices() {
  if (!state) return;
  $('#devices-page').innerHTML = `<div class="page-heading"><div><div class="eyebrow">AUTOMATISK IMPORT</div><h1>Din diktafon</h1><p>Anslut Sony ICD-UX570 via USB och registrera den för automatisk import.</p></div></div>
    <div class="device-grid">${state.devices.length ? state.devices.map(deviceCard).join('') : '<div class="settings-card"><span class="empty-icon">' + icons.device + '</span><h2>Väntar på din diktafon</h2><p>Anslut diktafonen till en USB-port. Du kan också importera en mapp från biblioteket.</p></div>'}</div>
    <p class="muted">Appen hittar REC_FILE i roten eller under PRIVATE/SONY och bevarar originalen på diktafonen. Import fortsätter när en registrerad enhet ansluts igen.</p>`;
  document.querySelectorAll<HTMLButtonElement>('[data-register]').forEach(b => b.onclick = () => void action(async () => { await invoke('register_device', { id: b.dataset.register }); toast('Diktafonen är registrerad'); await refresh(); }));
  document.querySelectorAll<HTMLButtonElement>('[data-unregister]').forEach(b => b.onclick = () => void action(async () => { await invoke('unregister_device', { id: b.dataset.unregister }); await refresh(); }));
  document.querySelectorAll<HTMLButtonElement>('[data-import-device]').forEach(b => b.onclick = () => void action(async () => {
    b.disabled = true;
    try { importMessage(await invoke<ImportReport>('import_device', { id: b.dataset.importDevice })); await refresh(); } finally { b.disabled = false; }
  }));
}
function deviceCard(d: Device) {
  return `<article class="settings-card"><div class="device-title"><span class="empty-icon">${icons.device}</span><span class="status ${d.block_path ? 'completed' : 'queued'}">${d.block_path ? 'Ansluten' : 'Frånkopplad'}</span></div><h2>${esc(d.model || 'Sony-diktafon')}</h2><p>${esc(d.label || d.vendor)}</p><dl><dt>Serienummer</dt><dd>${esc(d.serial || 'Saknas')}</dd><dt>Volym-ID</dt><dd>${esc(d.uuid || 'Saknas')}</dd><dt>Monterad på</dt><dd>${esc(d.mount_path || 'Inte monterad')}</dd></dl><div class="device-actions">${d.registered ? `<button class="secondary" data-unregister="${esc(d.id)}">Glöm enhet</button>` : `<button class="primary" data-register="${esc(d.id)}">Registrera diktafon</button>`}${d.block_path ? `<button class="secondary" data-import-device="${esc(d.id)}">Importera nu</button>` : ''}</div></article>`;
}
function renderSettings() {
  if (!state) return;
  const s = state.settings;
  $('#settings-page').innerHTML = `<div class="page-heading"><div><div class="eyebrow">DIN DATOR, DINA ORD</div><h1>Inställningar</h1><p>Välj din lokala Pythonmiljö och KB-Whisper-modell.</p></div></div><form id="settings-form" class="settings-card settings-form"><h2>Transkribering</h2><p>Installera Pythonmiljön och hämta modellen enligt README innan du aktiverar transkribering.</p><label>Pythonprogrammets sökväg<input name="python_path" value="${esc(s.python_path)}" required spellcheck="false" /></label><label>Modellmapp<input name="model_path" value="${esc(s.model_path)}" required spellcheck="false" /><small>Mappen ska innehålla model.bin och modellens konfigurationsfiler.</small></label><label>Batchstorlek<select name="batch_size">${[1, 2, 4, 8, 16].map(n => `<option ${n === s.batch_size ? 'selected' : ''}>${n}</option>`).join('')}</select><small>Starta med 8. Vid minnesbrist försöker workern med mindre batcher i FP16.</small></label><label class="checkbox-label"><input type="checkbox" name="transcription_enabled" ${s.transcription_enabled ? 'checked' : ''} />Aktivera transkribering</label><label class="checkbox-label"><input type="checkbox" name="auto_import" ${s.auto_import ? 'checked' : ''} />Importera automatiskt från registrerad diktafon</label><label class="checkbox-label"><input type="checkbox" name="start_at_login" ${s.start_at_login ? 'checked' : ''} />Starta i systemfältet vid inloggning (byggd app)</label><div class="settings-info">Svenska · KB-Whisper large · CUDA · FP16<br>Arkiv och databas: <code>${esc(state.data_dir)}</code><br>Worker-logg: <code>${esc(state.data_dir)}/logs/worker.log</code></div><button type="submit" class="primary">Spara inställningar</button></form><section id="comfy-settings" class="settings-card settings-form"></section>`;
  imagesView.showSettings();
  $('#settings-form').onsubmit = event => { event.preventDefault(); void action(async () => {
    const data = new FormData(event.target as HTMLFormElement);
    const settings: Settings = { python_path: String(data.get('python_path')).trim(), model_path: String(data.get('model_path')).trim(), batch_size: Number(data.get('batch_size')), transcription_enabled: data.has('transcription_enabled'), auto_import: data.has('auto_import'), start_at_login: data.has('start_at_login') };
    await invoke('save_settings', { settings }); toast('Inställningar sparade'); await refresh();
  }); };
}
function navigate(next: typeof page) {
  if (dirty && !window.confirm('Du har osparade rättningar. Lämna dem?')) return;
  if (page === 'library' && next !== page) { audioSource?.dispose(); audioSource = null; dirty = false; }
  if (page === 'images' && next !== page) imagesView.hide();
  if (page === 'library' && next !== page) selectionEvents?.abort();
  page = next;
  for (const name of ['library', 'queue', 'images', 'devices', 'settings']) $(`#${name}-page`).classList.toggle('hidden', name !== page);
  document.querySelectorAll<HTMLElement>('[data-page]').forEach(b => b.classList.toggle('active', b.dataset.page === page));
  $('#page-label').textContent = { library: 'Inspelningar', queue: 'Arbetskö', images: 'Bilder', devices: 'Diktafon', settings: 'Inställningar' }[page];
  if (page === 'devices') renderDevices();
  if (page === 'queue') renderQueue();
  if (page === 'images') imagesView.show();
  if (page === 'settings') renderSettings();
  if (page === 'library' && selected) void action(() => loadDetail(selected!));
}
document.querySelectorAll<HTMLButtonElement>('[data-page]').forEach(b => b.onclick = () => navigate(b.dataset.page as typeof page));
$('#import-open').onclick = () => $<HTMLDialogElement>('#import-dialog').showModal();
$('#import-cancel').onclick = () => $<HTMLDialogElement>('#import-dialog').close();
$('#import-form').onsubmit = event => { event.preventDefault(); if (busy) return; void action(async () => {
  if (!isTauri()) { toast('Starta desktopappen med npm run desktop för att importera filer.', true); return; }
  busy = true;
  const submit = $<HTMLButtonElement>('#import-form button[type=submit]'); submit.disabled = true; submit.textContent = 'Kopierar och verifierar …';
  try { const report = await invoke<ImportReport>('import_folder', { path: $<HTMLInputElement>('#import-path').value.trim() }); $<HTMLDialogElement>('#import-dialog').close(); importMessage(report); await refresh(); }
  finally { busy = false; submit.disabled = false; submit.textContent = 'Importera mapp'; }
}); };
let searchSequence = 0;
$('#search').oninput = () => {
  filter = $<HTMLInputElement>('#search').value.toLocaleLowerCase('sv').trim(); searchResults = null; renderList();
  const sequence = ++searchSequence;
  if (filter && isTauri()) void action(async () => {
    const ids = await invoke<string[]>('search_recordings', { query: filter });
    if (sequence === searchSequence) { searchResults = new Set(ids); renderList(); }
  });
};
window.addEventListener('beforeunload', event => { if (dirty) { event.preventDefault(); } });
window.addEventListener('pagehide', () => audioSource?.dispose());
if (isTauri()) { void refresh(); setInterval(() => void refresh(), 2000); }
else {
  const notice = $('#notice'); notice.classList.remove('hidden'); notice.textContent = 'Webbförhandsvisning. Starta desktopappen med npm run desktop för enhetsupptäckt, import och transkribering.';
}
