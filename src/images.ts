import { invoke, convertFileSrc, isTauri } from '@tauri-apps/api/core';

export interface ComfySettings {
  url: string; workflow: Record<string, { class_type: string; inputs: Record<string, unknown>; _meta?: { title?: string } }>;
  workflow_name: string; node_id: string; input_name: string;
}
export interface ImageDraft { prompt: string; source_text: string; recording_id: string | null; run_id: string | null }
export interface GeneratedImage { path: string; node_id: string; filename: string; subfolder: string; image_type: string }
export interface ImageJob { id: string; created_at: string; draft: ImageDraft; config: ComfySettings; status: string; prompt_id: string | null; error: string | null; images: GeneratedImage[]; release_pending: boolean }
interface ImageSnapshot { settings: ComfySettings; draft: ImageDraft; jobs: ImageJob[] }
export const emptyDraft = (): ImageDraft => ({ prompt: '', source_text: '', recording_id: null, run_id: null });
export const escapeHtml = (s: unknown) => String(s ?? '').replace(/[&<>"']/g, c => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' })[c]!);
export function workflowFields(workflow: unknown): { node: string; input: string; title: string }[] {
  if (!workflow || typeof workflow !== 'object' || Array.isArray(workflow)) throw new Error('Workflow måste vara ett JSON-objekt i API-format.');
  const entries = Object.entries(workflow);
  if (!entries.length || entries.some(([, n]) => !n || typeof n.class_type !== 'string' || !n.inputs || typeof n.inputs !== 'object' || Array.isArray(n.inputs))) {
    throw new Error('Exportera workflow i API-format från ComfyUI (Export API / Save API Format).');
  }
  return entries.flatMap(([id, n]) => Object.entries(n.inputs).filter(([, value]) => typeof value === 'string').map(([input]) => ({ node: id, input, title: `${n._meta?.title ?? n.class_type} · nod ${id} · ${input}` })));
}
export function selectedText(container: HTMLElement, selection: Selection | null): string {
  if (!selection || selection.isCollapsed || !selection.rangeCount) return '';
  const range = selection.getRangeAt(0);
  if (!container.contains(range.startContainer) || !container.contains(range.endContainer)) return '';
  return selection.toString().trim();
}
const statuses: Record<string, string> = { queued: 'Väntar på GPU', submitting: 'Skickar / bekräftar jobb', queued_comfy: 'I ComfyUI-kön', running: 'Skapar bild', downloading: 'Hämtar bilder', completed: 'Klar', failed: 'Misslyckad', uncertain: 'Behöver kontrolleras' };

export class ImagesView {
  private snapshot: ImageSnapshot | null = null;
  private draft: ImageDraft = emptyDraft();
  private draftVersion = 0;
  private persistedVersion = 0;
  private saving: Promise<void> | null = null;
  private saveTimer: ReturnType<typeof setTimeout> | undefined;
  private setup: ComfySettings | null = null;
  private visible = false;
  private disposed = false;
  private readonly esc = escapeHtml;
  constructor(private readonly container: HTMLElement, private readonly configContainer: () => HTMLElement | null, private readonly notify: (text: string, error?: boolean) => void, private readonly openRecording: (id: string, runId: string | null) => void) {}
  async refresh() {
    if (!isTauri()) return;
    const version = this.draftVersion;
    const snapshot = await invoke<ImageSnapshot>('image_snapshot');
    if (!this.snapshot && version === this.draftVersion) this.draft = snapshot.draft;
    this.snapshot = snapshot;
    if (this.visible) {
      if (!this.container.querySelector('#image-prompt')) this.render();
      this.renderJobs();
    }
  }
  show() { this.visible = true; this.render(); }
  hide() { this.visible = false; void this.persist().catch(e => this.notify(String(e), true)); }
  async fromSelection(text: string, recordingId: string, runId: string | null): Promise<boolean> {
    if (!this.snapshot) await this.refresh();
    if (this.draft.prompt.trim() && !window.confirm('Ersätta det befintliga bildutkastet med den markerade texten?')) return false;
    this.draft = { prompt: text, source_text: text, recording_id: recordingId, run_id: runId };
    this.draftVersion++;
    await this.persist();
    return true;
  }
  private change() {
    this.draftVersion++;
    clearTimeout(this.saveTimer);
    this.saveTimer = setTimeout(() => void this.persist().catch(e => this.notify(`Bildutkastet kunde inte sparas: ${e}`, true)), 400);
  }
  async persist(): Promise<void> {
    clearTimeout(this.saveTimer);
    if (!isTauri()) return;
    if (this.saving) { await this.saving; if (this.persistedVersion !== this.draftVersion) return this.persist(); return; }
    if (this.persistedVersion === this.draftVersion) return;
    const version = this.draftVersion;
    const draft = { ...this.draft };
    this.saving = invoke<void>('save_image_draft', { draft }).then(() => { this.persistedVersion = version; });
    try { await this.saving; } finally { this.saving = null; }
    if (!this.disposed && this.persistedVersion !== this.draftVersion) await this.persist();
  }
  private render() {
    const esc = this.esc;
    this.container.innerHTML = `<div class="page-heading"><div><div class="eyebrow">FRÅN ORD TILL BILD</div><h1>Bilder</h1><p>Markera text i ett transkript eller skriv en egen bildprompt.</p></div></div>
      <div class="settings-card image-editor"><label for="image-prompt">Bildprompt</label><textarea id="image-prompt" rows="7" placeholder="Beskriv bilden du vill skapa…">${esc(this.draft.prompt)}</textarea>
      <p id="image-source" class="muted">${this.draft.recording_id ? 'Utkast från ett transkript. Ändringar här påverkar inte transkriptet.' : 'Fristående bildprompt.'}</p>
      <div class="device-actions"><button id="generate-image" class="primary">Skapa bild</button><button id="clear-image-draft" class="secondary">Nytt utkast</button></div>
      <p class="muted">${this.snapshot?.settings.workflow_name ? `Workflow: ${esc(this.snapshot.settings.workflow_name)}. Whisper lämnar plats åt bildskapandet på GPU:n.` : 'Importera ditt ComfyUI-workflow och välj promptfält under Inställningar först.'}</p></div>
      <div id="image-jobs" aria-live="polite"></div>`;
    const input = this.container.querySelector<HTMLTextAreaElement>('#image-prompt')!;
    input.oninput = () => { this.draft.prompt = input.value; this.change(); this.updateCreateButton(); };
    this.container.querySelector<HTMLButtonElement>('#generate-image')!.onclick = () => void this.create();
    this.container.querySelector<HTMLButtonElement>('#clear-image-draft')!.onclick = () => {
      if (this.draft.prompt.trim() && !window.confirm('Rensa bildutkastet? Sparade bilder finns kvar.')) return;
      this.draft = emptyDraft(); this.change(); this.render();
    };
    this.updateCreateButton();
    this.renderJobs();
  }
  private updateCreateButton() {
    const button = this.container.querySelector<HTMLButtonElement>('#generate-image');
    if (button) button.disabled = !isTauri() || !this.draft.prompt.trim() || !this.snapshot?.settings.node_id;
  }
  private async create() {
    const button = this.container.querySelector<HTMLButtonElement>('#generate-image')!;
    button.disabled = true;
    try { await this.persist(); await invoke('create_image', { draft: { ...this.draft } }); this.notify('Bildjobbet har lagts i kön.'); await this.refresh(); }
    catch (e) { this.notify(String(e), true); }
    finally { this.updateCreateButton(); }
  }
  private renderJobs() {
    const target = this.container.querySelector<HTMLElement>('#image-jobs');
    if (!target) return;
    const esc = this.esc;
    const jobs = this.snapshot?.jobs ?? [];
    const html = jobs.length ? jobs.map(job => `<article class="settings-card image-job"><div class="image-job-head"><h2>${esc(new Date(job.created_at).toLocaleString('sv-SE'))}</h2><span class="status ${job.status === 'completed' ? 'completed' : job.status === 'failed' || job.status === 'uncertain' ? 'failed' : 'queued'}">${esc(statuses[job.status] ?? job.status)}</span></div>
      ${job.release_pending && ['completed', 'failed', 'uncertain'].includes(job.status) ? '<p class="muted">Väntar på ledig ComfyUI-kö och frigör GPU-minnet innan Whisper återstartas.</p>' : ''}
      ${job.error ? `<p class="inline-error">${esc(job.error)}</p>` : ''}
      <details><summary>Bildprompt och workflow</summary><p class="image-prompt-preview">${esc(job.draft.prompt)}</p><small>${esc(job.config.workflow_name)}</small></details>
      <div class="image-gallery">${job.images.map(image => `<a href="${esc(convertFileSrc(image.path))}" target="_blank" rel="noopener" data-image-preview="${esc(image.path)}"><img src="${esc(convertFileSrc(image.path))}" loading="lazy" alt="Bild skapad från den sparade bildprompten" /></a>`).join('')}</div>
      <div class="device-actions"><button class="secondary small" data-reuse-image="${esc(job.id)}">Använd prompt igen</button>${job.draft.recording_id ? `<button class="secondary small" data-image-source="${esc(job.id)}">Öppna inspelningen</button>` : ''}${job.prompt_id && ['failed', 'uncertain'].includes(job.status) && !job.release_pending ? `<button class="secondary small" data-follow-image="${esc(job.id)}">Följ upp / hämta igen</button>` : ''}</div>
      </article>`).join('') : '<div class="settings-card"><p class="muted">Här visas bildjobben och dina sparade bilder.</p></div>';
    // Preserve focus and image loading when the poll contains no changes.
    if (target.dataset.rendered === html) return;
    target.dataset.rendered = html; target.innerHTML = html;
    target.querySelectorAll<HTMLButtonElement>('[data-reuse-image]').forEach(button => button.onclick = () => {
      const job = jobs.find(j => j.id === button.dataset.reuseImage)!;
      if (this.draft.prompt.trim() && !window.confirm('Ersätta bildutkastet med denna prompt?')) return;
      this.draft = { ...job.draft }; this.change(); this.render();
    });
    target.querySelectorAll<HTMLButtonElement>('[data-image-source]').forEach(button => button.onclick = () => {
      const job = jobs.find(j => j.id === button.dataset.imageSource)!;
      this.openRecording(job.draft.recording_id!, job.draft.run_id);
    });
    target.querySelectorAll<HTMLButtonElement>('[data-follow-image]').forEach(button => button.onclick = () => void (async () => {
      button.disabled = true;
      try { await invoke('follow_image_job', { id: button.dataset.followImage }); await this.refresh(); }
      catch (e) { this.notify(String(e), true); button.disabled = false; }
    })());
    target.querySelectorAll<HTMLAnchorElement>('[data-image-preview]').forEach(link => link.onclick = event => {
      event.preventDefault();
      const dialog = document.createElement('dialog'); dialog.className = 'image-preview-dialog';
      dialog.innerHTML = `<button class="secondary">Stäng</button><img src="${esc(convertFileSrc(link.dataset.imagePreview!))}" alt="Skapad bild" />`;
      dialog.querySelector('button')!.onclick = () => dialog.close(); dialog.onclose = () => dialog.remove();
      document.body.append(dialog); dialog.showModal();
    });
  }
  showSettings() {
    const container = this.configContainer(); if (!container) return;
    this.setup = structuredClone(this.snapshot?.settings ?? { url: 'http://127.0.0.1:8188', workflow: {}, workflow_name: '', node_id: '', input_name: '' });
    const esc = this.esc;
    container.innerHTML = `<h2>ComfyUI – bildskapande</h2><p>Starta ComfyUI separat och importera ett workflow exporterat i API-format.</p><label>ComfyUI-adress<input id="comfy-url" value="${esc(this.setup.url)}" spellcheck="false" /></label><button type="button" id="comfy-test" class="secondary">Testa anslutning</button><label>Workflow i API-format<input id="comfy-workflow" type="file" accept=".json,application/json" /></label><p id="comfy-workflow-name" class="muted">${esc(this.setup.workflow_name || 'Inget workflow importerat')}</p><label>Fält som ska få bildprompten<select id="comfy-field"></select><small>Välj textfältet för positiv prompt. Negativ prompt och övriga workflow-inställningar behålls.</small></label><button type="button" id="comfy-save" class="primary">Spara bildinställningar</button>`;
    this.renderFields();
    container.querySelector<HTMLInputElement>('#comfy-workflow')!.onchange = event => void (async () => {
      const file = (event.target as HTMLInputElement).files?.[0]; if (!file) return;
      try {
        if (file.size > 5 * 1024 * 1024) throw new Error('Workflow-filen är större än 5 MB.');
        const workflow = JSON.parse(await file.text());
        const fields = workflowFields(workflow);
        if (!fields.length) throw new Error('Workflow saknar redigerbara textfält.');
        this.setup!.workflow = workflow; this.setup!.workflow_name = file.name;
        this.setup!.node_id = ''; this.setup!.input_name = '';
        container.querySelector('#comfy-workflow-name')!.textContent = file.name;
        this.renderFields();
      } catch (e) { this.notify(String(e), true); }
    })();
    container.querySelector<HTMLButtonElement>('#comfy-test')!.onclick = () => void (async () => {
      const button = container.querySelector<HTMLButtonElement>('#comfy-test')!; button.disabled = true;
      try { await invoke('test_comfy_connection', { url: container.querySelector<HTMLInputElement>('#comfy-url')!.value.trim() }); this.notify('Anslutningen till ComfyUI fungerar.'); }
      catch (e) { this.notify(String(e), true); } finally { button.disabled = false; }
    })();
    container.querySelector<HTMLButtonElement>('#comfy-save')!.onclick = () => void (async () => {
      const button = container.querySelector<HTMLButtonElement>('#comfy-save')!; button.disabled = true;
      try {
        const selection = JSON.parse(container.querySelector<HTMLSelectElement>('#comfy-field')!.value || 'null');
        if (!selection) throw new Error('Välj vilket textfält som ska få prompten.');
        this.setup!.url = container.querySelector<HTMLInputElement>('#comfy-url')!.value.trim();
        this.setup!.node_id = selection.node; this.setup!.input_name = selection.input;
        await invoke('save_comfy_settings', { settings: this.setup });
        await this.refresh(); this.notify('Bildinställningar sparade.');
      } catch (e) { this.notify(String(e), true); } finally { button.disabled = false; }
    })();
  }
  private renderFields() {
    const select = this.configContainer()?.querySelector<HTMLSelectElement>('#comfy-field'); if (!select || !this.setup) return;
    let fields: ReturnType<typeof workflowFields> = [];
    if (Object.keys(this.setup.workflow).length) fields = workflowFields(this.setup.workflow);
    select.innerHTML = '<option value="">Välj promptfält…</option>' + fields.map(field => `<option value="${this.esc(JSON.stringify({ node: field.node, input: field.input }))}" ${field.node === this.setup!.node_id && field.input === this.setup!.input_name ? 'selected' : ''}>${this.esc(field.title)}</option>`).join('');
  }
}
