import { invoke, convertFileSrc, isTauri } from '@tauri-apps/api/core';
import { escapeHtml as esc, showWorkflowEditor } from './images';
import type { ImageJob, SavedWorkflow } from './images';

export interface JournalEntry {
  id: string; version: number; title: string; date: string; kind: 'dream' | 'meditation'; language: 'sv' | 'en';
  description: string; reflections: string; description_revision: string; confirmed_revision: string | null;
  recording_id: string | null; run_id: string | null; favourite_image: string | null; created_at: string; updated_at: string;
}
export interface Detail { text: string; quote: string | null; english: string }
export interface Scene { title: string; summary: string; details: Detail[]; additions: Detail[]; questions: string[] }
export interface SceneDraft {
  id: string; entry_id: string; description_revision: string; parent_id: string | null; kind: string;
  scene: Scene; prompt: string; negative_prompt: string | null; manual: boolean; approved: boolean;
  position?: number; batch_created_at?: string; scene_ids: string[]; arrangement: string; provenance: unknown; created_at: string;
}
interface DraftingJob { id: string; entry_id: string; status: string; error: string | null; description_revision: string; result_ids: string[] }
interface DraftingSettings { binary_path: string; model_path: string }
interface DescriptionRevision { id: string; entry_id: string; description: string; language: string; created_at: string }
interface JournalSnapshot { entries: JournalEntry[]; drafts: SceneDraft[]; jobs: DraftingJob[]; descriptions: DescriptionRevision[]; settings: DraftingSettings; setup_ready: boolean }
interface ImageSnapshot { jobs: ImageJob[]; workflows: SavedWorkflow[]; settings: { selected_workflow_id: string | null } }
interface DraftEdit { id: string; scene: Scene; prompt: string; negative_prompt: string | null; manual: boolean; arrangement: string }

export const composeScene = (scene: Scene) => [...scene.details, ...scene.additions].map(d => d.english.trim().replace(/[. ]+$/, '')).filter(Boolean).join('. ');
export function latestDrafts(drafts: SceneDraft[]) {
  const parents = new Set(drafts.map(d => d.parent_id).filter(Boolean));
  return drafts.filter(d => !parents.has(d.id)).sort((a, b) => (b.batch_created_at ?? '').localeCompare(a.batch_created_at ?? '') || (a.position ?? 0) - (b.position ?? 0));
}
const today = () => new Date().toLocaleDateString('sv-SE');

export class JournalView {
  private snapshot: JournalSnapshot | null = null;
  private images: ImageSnapshot | null = null;
  private entry: JournalEntry | null = null;
  private visible = false;
  private entryDirty = false;
  private edits = new Map<string, DraftEdit>();
  private selected = new Set<string>();
  private aliases = new Map<string, string>();
  private timer: ReturnType<typeof setTimeout> | undefined;
  private serial: Promise<void> = Promise.resolve();
  private editSequence = 0;
  private signature = '';
  private workflowId: string | null = null;
  private working = false;
  private draftRequest: { key: string; id: string } | null = null;
  private imageRequest: { key: string; id: string } | null = null;
  constructor(private container: HTMLElement, private notify: (message: string, error?: boolean) => void, private openRecording: (id: string, run: string | null) => void) {}
  get dirty() { return this.entryDirty || this.edits.size > 0; }
  private resolve(key: string): string { return this.aliases.has(key) ? this.resolve(this.aliases.get(key)!) : key; }
  private async action(fn: () => Promise<void>) {
    if (this.working) return;
    this.working = true;
    try { await fn(); } catch (error) { this.notify(String(error), true); }
    finally { this.working = false; }
  }
  async refresh() {
    if (!isTauri()) return;
    const [snapshot, images] = await Promise.all([invoke<JournalSnapshot>('journal_snapshot'), invoke<ImageSnapshot>('image_snapshot')]);
    this.snapshot = snapshot; this.images = images;
    if (this.entry && !this.entryDirty) this.entry = snapshot.entries.find(e => e.id === this.entry!.id) ?? this.entry;
    if (this.visible) { this.renderList(); this.renderResults(); this.renderWorkflowChoice(); }
  }
  async show() {
    this.visible = true;
    if (!this.container.querySelector('#journal-detail')) {
      this.container.innerHTML = `<div class="page-heading"><div><div class="eyebrow">DINA INRE UPPLEVELSER</div><h1>Journal</h1><p>Bevara drömmar och meditationer i ord och bilder.</p></div><button id="new-entry" class="primary">Ny upplevelse</button></div>
        <div class="workspace journal-workspace"><section class="recording-panel"><label class="search-label"><input id="journal-search" type="search" placeholder="Sök i journalen…" aria-label="Sök i journalen" /></label><div id="journal-list"></div></section><section id="journal-detail" class="detail-panel"></section></div>`;
      this.container.querySelector<HTMLButtonElement>('#new-entry')!.onclick = () => void this.action(() => this.create());
      this.container.querySelector<HTMLInputElement>('#journal-search')!.oninput = () => this.renderList();
    }
    await this.refresh();
    if (!this.entry) this.entry = this.snapshot?.entries[0] ?? null;
    this.renderEditor();
  }
  hide() { this.visible = false; }
  async create(description = '', recordingId: string | null = null, runId: string | null = null, language: 'sv' | 'en' = 'sv') {
    await this.flush();
    if (!isTauri()) { this.notify('Öppna desktopappen för att spara journalposter.', true); return; }
    this.entry = await invoke<JournalEntry>('save_journal_entry', { input: { id: null, version: null, title: 'Ny upplevelse', date: today(), kind: 'dream', language, description, reflections: '', recording_id: recordingId, run_id: runId } });
    this.selected.clear(); this.edits.clear(); this.signature = '';
    await this.refresh(); if (this.visible) this.renderEditor();
  }
  private renderList() {
    const target = this.container.querySelector<HTMLElement>('#journal-list'); if (!target) return;
    const query = this.container.querySelector<HTMLInputElement>('#journal-search')?.value.toLocaleLowerCase() ?? '';
    const entries = this.snapshot?.entries.filter(e => `${e.title} ${e.description} ${e.reflections}`.toLocaleLowerCase().includes(query)) ?? [];
    target.innerHTML = entries.length ? entries.map(e => `<button class="recording-item ${this.entry?.id === e.id ? 'selected' : ''}" data-entry="${esc(e.id)}"><b>${esc(e.title)}</b><small>${esc(e.date)} · ${e.kind === 'dream' ? 'Dröm' : 'Meditation'} · ${e.language.toUpperCase()}</small></button>`).join('') : '<p class="muted journal-padding">Inga upplevelser ännu.</p>';
    target.querySelectorAll<HTMLButtonElement>('[data-entry]').forEach(button => button.onclick = () => void this.action(async () => {
      await this.flush(); this.entry = this.snapshot!.entries.find(e => e.id === button.dataset.entry)!;
      this.selected.clear(); this.signature = ''; this.renderEditor(); this.renderList();
    }));
  }
  private renderEditor() {
    const target = this.container.querySelector<HTMLElement>('#journal-detail'); if (!target) return;
    const entry = this.entry;
    if (!entry) { target.innerHTML = '<div class="detail-empty"><h2>Vad minns du?</h2><p>Skapa en post eller välj text från en inspelning.</p></div>'; return; }
    target.innerHTML = `<form id="journal-form" class="journal-editor">
      <label>Titel<input name="title" required value="${esc(entry.title)}" /></label>
      <div class="journal-metadata"><label>Datum<input name="date" type="date" required value="${esc(entry.date)}" /></label><label>Upplevelse<select name="kind"><option value="dream" ${entry.kind === 'dream' ? 'selected' : ''}>Dröm</option><option value="meditation" ${entry.kind === 'meditation' ? 'selected' : ''}>Meditation</option></select></label><label>Beskrivningens språk<select name="language"><option value="sv" ${entry.language === 'sv' ? 'selected' : ''}>Svenska</option><option value="en" ${entry.language === 'en' ? 'selected' : ''}>English</option></select></label></div>
      <label>Beskrivning<textarea name="description" rows="8" placeholder="Beskriv det du såg, kände och minns…">${esc(entry.description)}</textarea></label>
      ${entry.recording_id ? '<button type="button" id="entry-source" class="text-button">Öppna källinspelningen</button><p class="muted">Texten är en separat kopia. Granska och rätta den innan du skapar utkast.</p><button type="button" id="confirm-description" class="secondary">Bekräfta den granskade beskrivningen</button>' : ''}
      <label>Mina reflektioner<textarea name="reflections" rows="4" placeholder="Vad kan upplevelsen betyda för mig?">${esc(entry.reflections)}</textarea></label><p class="muted">Reflektionerna sparas separat och används bara när du uttryckligen väljer det nedan.</p>
      <div class="device-actions"><button class="secondary" type="submit">Spara nu</button><span id="journal-save-status" role="status">Sparad</span></div>
      <label class="checkbox-label"><input id="include-reflections" type="checkbox" />Ta med mina reflektioner i nästa utkast</label>
      <div class="device-actions"><button type="button" class="primary" id="draft-scenes">Skapa scenutkast</button><span id="confirmation-status" class="muted"></span></div>
    </form><section id="journal-results"></section>`;
    const form = target.querySelector<HTMLFormElement>('#journal-form')!;
    form.oninput = event => {
      if ((event.target as HTMLElement).id === 'include-reflections') return;
      this.entryDirty = true; this.editSequence++; this.status('Sparar …'); this.updateConfirmation(); this.updateButtons();
      clearTimeout(this.timer); this.timer = setTimeout(() => void this.flush().catch(e => { this.status('Kunde inte spara'); this.notify(String(e), true); }), 600);
    };
    form.onsubmit = event => { event.preventDefault(); void this.action(async () => { await this.flush(); this.renderResults(true); }); };
    target.querySelector<HTMLButtonElement>('#entry-source')?.addEventListener('click', () => void this.action(async () => { await this.flush(); this.openRecording(entry.recording_id!, entry.run_id); }));
    target.querySelector<HTMLButtonElement>('#confirm-description')?.addEventListener('click', () => void this.action(async () => {
      await this.flush(); this.entry = await invoke<JournalEntry>('confirm_description', { id: this.entry!.id, revision: this.entry!.description_revision }); this.updateConfirmation(); await this.refresh();
    }));
    target.querySelector<HTMLButtonElement>('#draft-scenes')!.onclick = () => void this.action(() => this.draft([]));
    this.updateConfirmation(); this.renderResults(true);
  }
  private status(text: string) { const element = this.container.querySelector('#journal-save-status'); if (element) element.textContent = text; }
  private updateConfirmation() {
    const e = this.entry; if (!e) return;
    const form = this.container.querySelector<HTMLFormElement>('#journal-form'); if (!form) return;
    const body = new FormData(form);
    const confirmed = e.confirmed_revision === e.description_revision && body.get('description') === e.description && body.get('language') === e.language;
    const label = this.container.querySelector('#confirmation-status'); if (label) label.textContent = confirmed ? 'Beskrivningen är redo för utkast' : 'Bekräfta beskrivningen före utkast';
    this.container.querySelector<HTMLButtonElement>('#draft-scenes')!.disabled = !confirmed || !String(body.get('description')).trim();
  }
  async flush() {
    clearTimeout(this.timer);
    this.serial = this.serial.catch(() => {}).then(async () => {
      if (this.entryDirty && this.entry) {
        const form = this.container.querySelector<HTMLFormElement>('#journal-form')!;
        const data = new FormData(form); const sequence = this.editSequence;
        const input = { id: this.entry.id, version: this.entry.version, title: String(data.get('title')), date: String(data.get('date')), kind: String(data.get('kind')), language: String(data.get('language')), description: String(data.get('description')), reflections: String(data.get('reflections')), recording_id: this.entry.recording_id, run_id: this.entry.run_id };
        const previousRevision = this.entry.description_revision;
        this.entry = await invoke<JournalEntry>('save_journal_entry', { input });
        if (previousRevision !== this.entry.description_revision) this.selected.clear();
        if (sequence === this.editSequence) this.entryDirty = false;
        this.status(this.entryDirty ? 'Osparade ändringar' : 'Sparad'); this.updateConfirmation();
      }
      for (const [key, edit] of [...this.edits]) {
        const fingerprint = JSON.stringify(edit);
        const previousId = edit.id;
        const copy = structuredClone(edit);
        copy.scene.additions = copy.scene.additions.filter(d => d.english.trim());
        const saved = await invoke<SceneDraft>('save_scene_draft', { edit: copy });
        this.aliases.set(key, saved.id);
        if (previousId !== key) this.aliases.set(previousId, saved.id);
        if (this.selected.delete(previousId) || this.selected.delete(key)) this.selected.add(saved.id);
        if (JSON.stringify(edit) === fingerprint) this.edits.delete(key);
        else edit.id = saved.id;
        this.snapshot?.drafts.unshift(saved);
      }
    });
    await this.serial;
    // Edits made during an in-flight save must also be durable before navigation/action.
    if (this.entryDirty || this.edits.size) await this.flush();
    await this.refresh();
  }
  private async draft(sceneIds: string[]) {
    await this.flush();
    const e = this.entry!;
    const include = this.container.querySelector<HTMLInputElement>('#include-reflections')?.checked ?? false;
    const key = JSON.stringify([e.id, e.description_revision, include ? e.reflections : null, sceneIds]);
    if (this.draftRequest?.key !== key) this.draftRequest = { key, id: crypto.randomUUID() };
    await invoke('request_drafting', { requestId: this.draftRequest!.id, entryId: e.id, revision: e.description_revision, includeReflections: include, sceneIds });
    this.draftRequest = null;
    const checkbox = this.container.querySelector<HTMLInputElement>('#include-reflections'); if (checkbox) checkbox.checked = false;
    this.notify('Scenutkastet har lagts i kön.'); await this.refresh();
  }
  private async generate(keys: string[]) {
    await this.flush(); const e = this.entry!;
    const ids = keys.map(key => this.resolve(key));
    const workflowId = this.container.querySelector<HTMLSelectElement>('#journal-workflow')!.value;
    const key = JSON.stringify([e.id, ids, workflowId]);
    if (this.imageRequest?.key !== key) this.imageRequest = { key, id: crypto.randomUUID() };
    await invoke('generate_journal_images', { requestId: this.imageRequest!.id, entryId: e.id, draftIds: ids, workflowId });
    this.imageRequest = null;
    this.notify('Bildjobben har lagts i kön.'); await this.refresh();
  }
  private renderResults(force = false) {
    const target = this.container.querySelector<HTMLElement>('#journal-results');
    const e = this.entry; if (!target || !e || !this.snapshot) return;
    const drafts = this.snapshot.drafts.filter(d => d.entry_id === e.id);
    const current = latestDrafts(drafts).filter(d => d.description_revision === e.description_revision);
    const jobs = this.snapshot.jobs.filter(j => j.entry_id === e.id);
    const images = this.images?.jobs.filter(j => j.entry_id === e.id) ?? [];
    const signature = JSON.stringify([e.description_revision, e.favourite_image, drafts, jobs, images, this.images?.workflows]);
    // Polling never replaces a prompt while it is being edited.
    if (!force && (this.edits.size || target.querySelector<HTMLTextAreaElement>('#manual-prompt')?.value || signature === this.signature)) return;
    this.signature = signature;
    const workflows = this.images?.workflows ?? [];
    const selectedWorkflow = this.workflowId ?? this.images?.settings.selected_workflow_id;
    const revisions = this.snapshot.descriptions.filter(r => r.entry_id === e.id);
    target.innerHTML = `<div class="journal-review"><h2>Scener och bildpromptar</h2><p class="muted">Granska detaljer och kreativa tillägg. Bildprompten är på engelska och kan ändras. Ett citat hjälper dig att kontrollera källan; granska även modellens tolkning.</p>
      ${!this.snapshot.setup_ready ? '<p class="notice">Språkmodellen behöver installeras. Se README och Inställningar. Du kan alltid skriva en egen prompt.</p>' : ''}
      <label>Bildflöde<select id="journal-workflow"><option value="">Välj sparat ComfyUI-flöde…</option>${workflows.map(w => `<option value="${esc(w.id)}" ${w.id === selectedWorkflow ? 'selected' : ''}>${esc(w.name)}</option>`).join('')}</select></label>
      <div class="device-actions"><button id="journal-add-workflow" class="secondary">Lägg till flöde</button></div>
      <p id="journal-workflow-hint" class="muted"></p>
      <div class="device-actions"><button id="combine-scenes" class="secondary">Kombinera valda scener</button><button id="generate-all-scenes" class="primary">Generera alla scener separat</button></div>
      <div id="scene-cards">${current.map(d => this.card(d)).join('') || '<p class="muted">Skapa scenutkast eller skriv en egen prompt nedan.</p>'}</div>
      <details><summary>Skriv en egen scenprompt</summary><label>Engelsk bildprompt<textarea id="manual-prompt" rows="4"></textarea></label><button id="manual-scene" class="secondary">Spara scen för granskning</button></details>
      <section><h3>Utkastjobb</h3>${jobs.map(j => `<article class="settings-card"><b>${esc(({ queued: 'I kön', running: 'Skapar utkast', completed: 'Klart', failed: 'Misslyckat' } as Record<string, string>)[j.status] ?? j.status)}</b>${j.description_revision !== e.description_revision ? '<p class="muted">Tillhör en tidigare beskrivning.</p>' : ''}${j.error ? `<p class="inline-error">${esc(j.error)}</p><button data-retry-draft="${esc(j.id)}" class="secondary small">Försök igen</button>` : ''}</article>`).join('') || '<p class="muted">Inga utkastjobb ännu.</p>'}</section>
      <section><h3>Bilder och försök</h3>${images.map(job => `<article class="settings-card"><b>${esc(job.config.workflow_name)} · ${esc(({queued:'I kön',submitting:'Skickar',queued_comfy:'I ComfyUI-kön',running:'Skapar bild',downloading:'Hämtar',completed:'Klar',failed:'Misslyckad',uncertain:'Osäker'} as Record<string,string>)[job.status] ?? job.status)}</b>${job.error ? `<p class="inline-error">${esc(job.error)}</p>` : ''}<details><summary>Exakt bildprompt och flöde</summary><p class="image-prompt-preview">${esc(job.draft.prompt)}</p><p>${esc(job.draft.negative_prompt ?? '')}</p><pre>${esc(JSON.stringify(job.config.workflow, null, 2))}</pre></details><div class="image-gallery">${job.images.filter(i => !i.deleted).map(i => `<div><button class="image-thumbnail" data-preview="${esc(i.path)}"><img src="${esc(convertFileSrc(i.path))}" alt="${esc(job.draft.prompt)}" loading="lazy" /></button><button class="secondary small" data-favourite="${esc(i.path)}">${e.favourite_image === i.path ? '★ Favorit · avmarkera' : '☆ Välj favorit'}</button><button class="text-button" data-delete-image="${esc(i.path)}" data-job="${esc(job.id)}">Ta bort bild</button></div>`).join('')}</div>${job.prompt_id && ['failed','uncertain'].includes(job.status) && !job.release_pending ? `<button data-follow-image="${esc(job.id)}" class="secondary small">Följ upp / hämta igen</button>` : ''}${job.release_pending && job.error ? `<button data-dismiss-image="${esc(job.id)}" class="secondary small">Avsluta uppföljning</button>` : ''}</article>`).join('') || '<p class="muted">Inga bilder ännu.</p>'}</section>
      <details><summary>Tidigare beskrivningar och utkast</summary>${revisions.map(r => `<article><b>${esc(new Date(r.created_at).toLocaleString('sv-SE'))} · ${esc(r.language)}</b><p class="image-prompt-preview">${esc(r.description)}</p></article>`).join('')}${drafts.filter(d => !current.some(c => c.id === d.id)).map(d => `<article><b>${esc(d.scene.title)} · ${esc(new Date(d.created_at).toLocaleString('sv-SE'))}</b><p class="image-prompt-preview">${esc(d.prompt)}</p></article>`).join('')}</details></div>`;
    target.querySelector<HTMLButtonElement>('#journal-add-workflow')!.onclick = () => {
      showWorkflowEditor(this.notify, async id => { this.workflowId = id; await this.refresh(); });
    };
    this.renderWorkflowChoice();
    target.querySelector<HTMLButtonElement>('#manual-scene')!.onclick = () => void this.action(async () => {
      const prompt = target.querySelector<HTMLTextAreaElement>('#manual-prompt')!.value;
      await this.flush(); await invoke('manual_scene', { entryId: this.entry!.id, revision: this.entry!.description_revision, prompt }); target.querySelector<HTMLTextAreaElement>('#manual-prompt')!.value = ''; await this.refresh();
    });
    target.querySelector<HTMLButtonElement>('#combine-scenes')!.onclick = () => void this.action(async () => { await this.flush(); await this.draft([...this.selected].map(key => this.resolve(key))); });
    target.querySelector<HTMLButtonElement>('#generate-all-scenes')!.onclick = () => void this.action(() => this.generate(current.filter(d => d.kind === 'scene').map(d => d.id)));
    current.forEach(d => this.bindCard(target, d));
    target.querySelectorAll<HTMLButtonElement>('[data-retry-draft]').forEach(b => b.onclick = () => void this.action(async () => {
      const job = this.snapshot!.jobs.find(j => j.id === b.dataset.retryDraft)!;
      const raw = job as DraftingJob & { scene_ids: string[] };
      await this.draft(raw.scene_ids.map(key => this.resolve(key)));
    }));
    target.querySelectorAll<HTMLButtonElement>('[data-favourite]').forEach(b => b.onclick = () => void this.action(async () => {
      await this.flush(); this.entry = await invoke('favourite_image', { entryId: e.id, path: this.entry!.favourite_image === b.dataset.favourite ? null : b.dataset.favourite }); await this.refresh();
    }));
    target.querySelectorAll<HTMLButtonElement>('[data-delete-image]').forEach(b => b.onclick = () => void this.action(async () => {
      if (!window.confirm('Ta bort den lokala bilden?')) return;
      await this.flush(); await invoke('delete_generated_image', { id: b.dataset.job, path: b.dataset.deleteImage }); await this.refresh();
    }));
    for (const [selector, command, dataset] of [['[data-follow-image]','follow_image_job','followImage'],['[data-dismiss-image]','dismiss_image_job','dismissImage']] as const) {
      target.querySelectorAll<HTMLButtonElement>(selector).forEach(b => b.onclick = () => void this.action(async () => {
        if (command === 'dismiss_image_job' && !window.confirm('Bekräfta att ComfyUI inte kör jobbet. Uppföljningen avslutas utan att jobbet avbryts på servern.')) return;
        await invoke(command, { id: b.dataset[dataset] }); await this.refresh();
      }));
    }
    target.querySelectorAll<HTMLButtonElement>('[data-preview]').forEach(b => b.onclick = () => {
      const dialog = document.createElement('dialog'); dialog.className = 'image-preview-dialog';
      dialog.innerHTML = `<button class="secondary">Stäng</button><img src="${esc(convertFileSrc(b.dataset.preview!))}" alt="Sparad bild" />`;
      dialog.querySelector('button')!.onclick = () => dialog.close(); dialog.onclose = () => dialog.remove(); document.body.append(dialog); dialog.showModal();
    });
    this.updateButtons();
  }
  private renderWorkflowChoice() {
    const select = this.container.querySelector<HTMLSelectElement>('#journal-workflow');
    if (!select) return;
    const workflows = this.images?.workflows ?? [];
    const selected = this.workflowId ?? this.images?.settings.selected_workflow_id;
    const signature = JSON.stringify([workflows.map(w => [w.id, w.name]), selected]);
    if (select.dataset.rendered !== signature) {
      select.dataset.rendered = signature;
      select.innerHTML = '<option value="">Välj sparat ComfyUI-flöde…</option>' + workflows.map(w => `<option value="${esc(w.id)}" ${w.id === selected ? 'selected' : ''}>${esc(w.name)}</option>`).join('');
    }
    const hint = this.container.querySelector<HTMLElement>('#journal-workflow-hint');
    if (hint) hint.textContent = workflows.length ? 'Flöden delas med Bilder och sparas lokalt. Välj flöde för nästa generation.' : 'Välj Lägg till flöde och importera ComfyUI-JSON i API-format innan du genererar.';
    this.updateButtons();
  }
  private card(d: SceneDraft) {
    const detail = (items: Detail[], group: string) => items.map((item, i) => `<div class="journal-detail-row" data-group="${group}" data-index="${i}"><label>${group === 'details' ? 'Detalj från beskrivningen' : 'Kreativt tillägg'}<input data-field="text" value="${esc(item.text)}" /></label>${item.quote ? `<blockquote>${esc(item.quote)}</blockquote>` : ''}<label>Engelsk promptdel<textarea rows="2" data-field="english">${esc(item.english)}</textarea></label>${group === 'additions' ? '<button class="text-button" data-remove-addition>Ta bort tillägg</button>' : ''}</div>`).join('');
    return `<article class="settings-card scene-card" data-scene="${esc(d.id)}"><div class="image-job-head"><h3>${esc(d.scene.title)} ${d.kind === 'composition' ? '· Komposition' : ''}</h3>${d.kind === 'scene' ? `<label class="checkbox-label"><input data-select-scene type="checkbox" ${this.selected.has(d.id) ? 'checked' : ''} />Kombinera</label>` : ''}</div><label>Scenens titel<input data-title value="${esc(d.scene.title)}" /></label><label>Sammanfattning<textarea data-summary rows="2">${esc(d.scene.summary)}</textarea></label>${d.kind === 'composition' ? `<label>Hur scenerna passar ihop<textarea data-arrangement rows="2">${esc(d.arrangement)}</textarea></label>` : ''}${d.scene.questions.map(q => `<p class="notice">${esc(q)} Du kan behålla tvetydigheten eller rätta beskrivningen.</p>`).join('')}
      <details><summary>Detaljer och kreativa tillägg</summary>${detail(d.scene.details,'details')}${detail(d.scene.additions,'additions')}</details><label>Bildprompt på engelska<textarea data-prompt rows="5">${esc(d.prompt)}</textarea></label><label>Negativ prompt (valfri)<textarea data-negative rows="2" placeholder="Lämna orört för flödets standard">${esc(d.negative_prompt ?? '')}</textarea></label>
      <p data-review-status class="muted">${d.approved ? 'Godkänd för generation' : 'Behöver granskas'}${d.manual ? ' · Manuellt redigerad prompt' : ''}</p><div class="device-actions"><button data-recompose class="text-button">Bygg prompt från detaljer igen</button><button data-save class="secondary small">Spara ny version</button><button data-approve class="secondary small">Godkänn granskad prompt</button><button data-generate class="primary small">Generera bild</button></div></article>`;
  }
  private bindCard(target: HTMLElement, draft: SceneDraft) {
    const card = target.querySelector<HTMLElement>(`[data-scene="${draft.id}"]`)!;
    const edit: DraftEdit = { id: draft.id, scene: structuredClone(draft.scene), prompt: draft.prompt, negative_prompt: draft.negative_prompt, manual: draft.manual, arrangement: draft.arrangement };
    const changed = () => { this.edits.set(draft.id, edit); card.querySelector('[data-review-status]')!.textContent = 'Ändrad · spara och granska igen'; this.updateButtons(); };
    card.querySelector<HTMLInputElement>('[data-title]')!.oninput = event => { edit.scene.title = (event.target as HTMLInputElement).value; changed(); };
    card.querySelector<HTMLTextAreaElement>('[data-summary]')!.oninput = event => { edit.scene.summary = (event.target as HTMLTextAreaElement).value; changed(); };
    card.querySelector<HTMLTextAreaElement>('[data-arrangement]')?.addEventListener('input', event => { edit.arrangement = (event.target as HTMLTextAreaElement).value; changed(); });
    const prompt = card.querySelector<HTMLTextAreaElement>('[data-prompt]')!;
    prompt.oninput = () => { edit.prompt = prompt.value; edit.manual = true; changed(); };
    card.querySelector<HTMLTextAreaElement>('[data-negative]')!.oninput = event => { edit.negative_prompt = (event.target as HTMLTextAreaElement).value; changed(); };
    card.querySelectorAll<HTMLElement>('[data-group]').forEach(row => {
      const group = row.dataset.group as 'details' | 'additions'; const index = Number(row.dataset.index);
      row.querySelectorAll<HTMLInputElement | HTMLTextAreaElement>('[data-field]').forEach(input => input.oninput = () => {
        edit.scene[group][index][input.dataset.field as 'text' | 'english'] = input.value;
        if (!edit.manual) { edit.prompt = composeScene(edit.scene); prompt.value = edit.prompt; } changed();
      });
      row.querySelector<HTMLButtonElement>('[data-remove-addition]')?.addEventListener('click', () => {
        const item = edit.scene[group][index];
        const fragment = item.english;
        const needsManualRemoval = edit.manual && !edit.prompt.includes(fragment);
        if (edit.manual) edit.prompt = edit.prompt.replace(fragment, '').trim();
        item.english = ''; item.text = ''; row.remove();
        if (!edit.manual) edit.prompt = composeScene(edit.scene);
        prompt.value = edit.prompt; changed();
        if (needsManualRemoval) card.querySelector('[data-review-status]')!.textContent = 'Tillägget är borttaget från listan. Granska din manuella prompt och ta bort det där också.';
      });
    });
    card.querySelector<HTMLInputElement>('[data-select-scene]')?.addEventListener('change', event => {
      if ((event.target as HTMLInputElement).checked) this.selected.add(draft.id); else this.selected.delete(draft.id); this.updateButtons();
    });
    card.querySelector<HTMLButtonElement>('[data-recompose]')!.onclick = () => { edit.manual = false; edit.prompt = composeScene(edit.scene); prompt.value = edit.prompt; changed(); };
    card.querySelector<HTMLButtonElement>('[data-save]')!.onclick = () => void this.action(async () => { await this.flush(); this.renderResults(true); });
    card.querySelector<HTMLButtonElement>('[data-approve]')!.onclick = () => void this.action(async () => { await this.flush(); await invoke('approve_scene', { id: this.resolve(draft.id) }); await this.refresh(); });
    card.querySelector<HTMLButtonElement>('[data-generate]')!.onclick = () => void this.action(() => this.generate([draft.id]));
  }
  private updateButtons() {
    const target = this.container.querySelector('#journal-results'); if (!target) return;
    const current = latestDrafts(this.snapshot?.drafts.filter(d => d.entry_id === this.entry?.id) ?? []).filter(d => d.description_revision === this.entry?.description_revision);
    const workflow = target.querySelector<HTMLSelectElement>('#journal-workflow');
    const canGenerate = !!workflow?.value && !this.edits.size && !this.entryDirty;
    if (workflow) workflow.onchange = () => { this.workflowId = workflow.value; this.updateButtons(); };
    target.querySelectorAll<HTMLButtonElement>('[data-generate]').forEach(button => {
      const key = (button.closest('[data-scene]') as HTMLElement).dataset.scene;
      button.disabled = !canGenerate || !current.find(d => d.id === key)?.approved;
    });
    const scenes = current.filter(d => d.kind === 'scene');
    target.querySelector<HTMLButtonElement>('#generate-all-scenes')!.disabled = !canGenerate || !scenes.length || scenes.some(d => !d.approved);
    target.querySelector<HTMLButtonElement>('#combine-scenes')!.disabled = this.selected.size < 2;
  }
  renderSettings(container: HTMLElement) {
    const settings = this.snapshot?.settings; if (!settings) return;
    container.innerHTML = `<h2>Lokal språkmodell för scenutkast</h2><p>Kör <code>bash scripts/setup-drafting.sh</code> en gång enligt README. Utkast skapas lokalt. Saknade modeller hämtas aldrig automatiskt under bearbetning.</p><form id="drafting-settings-form"><label>llama-server<input name="binary_path" value="${esc(settings.binary_path)}" required spellcheck="false" /></label><label>GGUF-modellfil<input name="model_path" value="${esc(settings.model_path)}" required spellcheck="false" /></label><p role="status">${this.snapshot?.setup_ready ? 'Program och modellfil finns.' : 'Program eller modellfil saknas.'} GPU- och modellstart verifieras när ett utkast körs.</p><button class="secondary">Spara språkmodell</button></form>`;
    container.querySelector<HTMLFormElement>('form')!.onsubmit = event => { event.preventDefault(); void this.action(async () => {
      const data = new FormData(event.target as HTMLFormElement); await invoke('save_drafting_settings', { settings: { binary_path: String(data.get('binary_path')).trim(), model_path: String(data.get('model_path')).trim() } }); this.notify('Språkmodellens inställningar sparade.'); await this.refresh(); this.renderSettings(container);
    }); };
  }
}
