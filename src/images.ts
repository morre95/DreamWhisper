import { invoke, convertFileSrc, isTauri } from '@tauri-apps/api/core';

export interface PromptField { node_id: string; input_name: string }
export interface ComfySettings {
  url: string; workflow: Record<string, { class_type: string; inputs: Record<string, unknown>; _meta?: { title?: string } }>;
  workflow_name: string; node_id: string; input_name: string; negative_field: PromptField | null;
  selected_workflow_id: string | null; comfy_directory: string; comfy_python_path: string;
}
export interface ImageDraft { prompt: string; negative_prompt: string | null; source_text: string; recording_id: string | null; run_id: string | null }
export interface GeneratedImage { deleted?: boolean; created_at?: string | null; path: string; node_id: string; filename: string; subfolder: string; image_type: string }
export interface ImageJob { entry_id?: string | null; draft_revision_id?: string | null; id: string; created_at: string; draft: ImageDraft; config: ComfySettings; status: string; prompt_id: string | null; error: string | null; images: GeneratedImage[]; release_pending: boolean }
export interface SavedWorkflow { id: string; name: string; workflow: ComfySettings['workflow']; node_id: string; input_name: string; negative_field: PromptField | null }
interface ImageSnapshot { workflows: SavedWorkflow[]; settings: ComfySettings; draft: ImageDraft; jobs: ImageJob[] }
export const emptyDraft = (): ImageDraft => ({ prompt: '', negative_prompt: null, source_text: '', recording_id: null, run_id: null });
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
export function galleryImages(jobs: ImageJob[]) {
  return jobs.flatMap(job => [...job.images].reverse().filter(image => !image.deleted).map(image => ({ job, image })))
    .sort((a, b) => Date.parse(b.image.created_at ?? b.job.created_at) - Date.parse(a.image.created_at ?? a.job.created_at));
}
const statuses: Record<string, string> = { queued: 'Väntar på GPU', submitting: 'Skickar / bekräftar jobb', queued_comfy: 'I ComfyUI-kön', running: 'Skapar bild', downloading: 'Hämtar bilder', completed: 'Klar', failed: 'Misslyckad', uncertain: 'Behöver kontrolleras' };

export class ImagesView {
  private snapshot: ImageSnapshot | null = null;
  private draft: ImageDraft = emptyDraft();
  private draftVersion = 0;
  private persistedVersion = 0;
  private saving: Promise<void> | null = null;
  private saveTimer: ReturnType<typeof setTimeout> | undefined;
  private workflowChanging = false;
  private serverStarting = false;
  private visible = false;
  private galleryVisible = false;
  private creating = false;
  private readonly esc = escapeHtml;
  constructor(private readonly container: HTMLElement, private readonly configContainer: () => HTMLElement | null, private readonly notify: (text: string, error?: boolean) => void, private readonly openRecording: (id: string, runId: string | null) => void, private readonly galleryContainer?: HTMLElement) {}
  async refresh() {
    if (!isTauri()) return;
    const version = this.draftVersion;
    const snapshot = await invoke<ImageSnapshot>('image_snapshot');
    if (!this.snapshot && version === this.draftVersion) this.draft = { ...emptyDraft(), ...snapshot.draft };
    this.snapshot = snapshot;
    if (this.visible) {
      if (!this.container.querySelector('#image-prompt')) this.render();
      this.renderWorkflows();
      this.updateNegativePrompt();
      this.renderJobs();
      this.updateCreateButton();
    }
    if (this.galleryVisible) this.renderGallery();
    this.renderSavedWorkflowSettings();
    await this.refreshServerStatus();
  }
  showGallery() { this.galleryVisible = true; this.renderGallery(); }
  hideGallery() { this.galleryVisible = false; }
  show() { this.visible = true; this.render(); }
  hide() { this.visible = false; void this.persist().catch(e => this.notify(String(e), true)); }
  async fromSelection(text: string, recordingId: string, runId: string | null): Promise<boolean> {
    if (!this.snapshot) await this.refresh();
    if ((this.draft.prompt.trim() || this.draft.negative_prompt?.trim()) && !window.confirm('Ersätta det befintliga bildutkastet med den markerade texten?')) return false;
    this.draft = { prompt: text, negative_prompt: null, source_text: text, recording_id: recordingId, run_id: runId };
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
    if (this.persistedVersion !== this.draftVersion) await this.persist();
  }
  private render() {
    const esc = this.esc;
    this.container.innerHTML = `<div class="page-heading"><div><div class="eyebrow">FRÅN ORD TILL BILD</div><h1>Bilder</h1><p>Markera text i ett transkript eller skriv en egen bildprompt.</p></div></div>
      <section id="workflow-library" class="settings-card workflow-library"></section>
      <div class="settings-card image-editor"><label for="image-prompt">Positiv prompt</label><textarea id="image-prompt" rows="7" placeholder="Beskriv bilden du vill skapa…">${esc(this.draft.prompt)}</textarea>
      <label for="image-negative-prompt" class="negative-prompt-label">Negativ prompt</label><textarea id="image-negative-prompt" rows="3" placeholder="Beskriv det du vill undvika i bilden…"></textarea><small id="negative-prompt-hint" class="muted"></small>
      <p id="image-source" class="muted">${this.draft.recording_id ? 'Utkast från ett transkript. Ändringar här påverkar inte transkriptet.' : 'Fristående bildprompt.'}</p>
      <div class="device-actions"><button id="generate-image" class="primary">Skapa bild</button><button id="clear-image-draft" class="secondary">Nytt utkast</button></div>
      <p class="muted">Whisper lämnar plats åt bildskapandet på GPU:n. ComfyUI kan startas under Inställningar.</p></div>
      <section><h2>De fem senaste bilderna</h2><div id="latest-images"></div></section><div id="image-jobs" aria-live="polite"></div>`;
    const input = this.container.querySelector<HTMLTextAreaElement>('#image-prompt')!;
    input.oninput = () => { this.draft.prompt = input.value; this.change(); this.updateCreateButton(); };
    this.container.querySelector<HTMLTextAreaElement>('#image-negative-prompt')!.oninput = event => {
      this.draft.negative_prompt = (event.target as HTMLTextAreaElement).value; this.change();
    };
    this.container.querySelector<HTMLButtonElement>('#generate-image')!.onclick = () => void this.create();
    this.container.querySelector<HTMLButtonElement>('#clear-image-draft')!.onclick = () => {
      if ((this.draft.prompt.trim() || this.draft.negative_prompt?.trim()) && !window.confirm('Rensa bildutkastet? Sparade bilder finns kvar.')) return;
      this.draft = emptyDraft(); this.change(); this.render();
    };
    this.renderWorkflows();
    this.updateNegativePrompt();
    this.updateCreateButton();
    this.renderJobs();
  }
  private updateNegativePrompt() {
    const input = this.container.querySelector<HTMLTextAreaElement>('#image-negative-prompt');
    if (!input) return;
    const workflow = this.snapshot?.workflows?.find(w => w.id === this.snapshot?.settings.selected_workflow_id);
    const mapping = workflow?.negative_field;
    input.disabled = !mapping;
    const original = mapping ? workflow?.workflow[mapping.node_id]?.inputs[mapping.input_name] : '';
    const value = this.draft.negative_prompt ?? (typeof original === 'string' ? original : '');
    if (input.value !== value) input.value = value;
    this.container.querySelector<HTMLElement>('#negative-prompt-hint')!.textContent = mapping ? 'Förifyllt från flödet tills du ändrar det. Töm fältet om du vill använda en tom negativ prompt.' : 'Välj ett negativt promptfält via Ändra namn / promptfält för detta flöde.';
  }
  private updateCreateButton() {
    const button = this.container.querySelector<HTMLButtonElement>('#generate-image');
    if (button) button.disabled = this.creating || this.workflowChanging || !isTauri() || !this.draft.prompt.trim() || !this.snapshot?.workflows?.some(w => w.id === this.snapshot?.settings.selected_workflow_id);
  }
  private async create() {
    if (this.creating || this.workflowChanging) return;
    this.creating = true;
    const draft = { ...this.draft };
    const workflowId = this.snapshot?.settings.selected_workflow_id;
    const button = this.container.querySelector<HTMLButtonElement>('#generate-image')!;
    button.disabled = true;
    try { await this.persist(); await invoke('create_image', { draft, workflowId }); this.notify('Bildjobbet har lagts i kön.'); await this.refresh(); }
    catch (e) { this.notify(String(e), true); }
    finally { this.creating = false; this.updateCreateButton(); }
  }
  private renderJobs() {
    const target = this.container.querySelector<HTMLElement>('#image-jobs');
    if (!target) return;
    const esc = this.esc;
    this.renderImageCards(this.container.querySelector<HTMLElement>('#latest-images')!, 5);
    const jobs = this.snapshot?.jobs ?? [];
    const html = jobs.length ? jobs.map(job => `<article class="settings-card image-job"><div class="image-job-head"><h2>${esc(new Date(job.created_at).toLocaleString('sv-SE'))}</h2><span class="status ${job.status === 'completed' ? 'completed' : job.status === 'failed' || job.status === 'uncertain' ? 'failed' : 'queued'}">${esc(statuses[job.status] ?? job.status)}</span></div>
      ${job.release_pending && ['completed', 'failed', 'uncertain'].includes(job.status) ? '<p class="muted">Väntar på ledig ComfyUI-kö och frigör GPU-minnet innan Whisper återstartas.</p>' : ''}
      ${job.error ? `<p class="inline-error">${esc(job.error)}</p>` : ''}
      <details><summary>Bildprompt och workflow</summary><p class="image-prompt-preview">${esc(job.draft.prompt)}</p>${job.draft.negative_prompt != null ? `<b>Negativ prompt</b><p class="image-prompt-preview">${esc(job.draft.negative_prompt) || '(tom)'}</p>` : ''}<small>${esc(job.config.workflow_name)}</small></details>
      ${job.images.some(image => !image.deleted) ? `<div class="image-gallery">${job.images.filter(image => !image.deleted).map(image => this.thumbnail(image, job)).join('')}</div>` : ''}
      <div class="device-actions"><button class="secondary small" data-reuse-image="${esc(job.id)}">Använd prompt igen</button>${job.draft.recording_id ? `<button class="secondary small" data-image-source="${esc(job.id)}">Öppna inspelningen</button>` : ''}${job.prompt_id && ['failed', 'uncertain'].includes(job.status) && !job.release_pending ? `<button class="secondary small" data-follow-image="${esc(job.id)}">Följ upp / hämta igen</button>` : ''}${job.release_pending && job.error ? `<button class="secondary small" data-dismiss-image="${esc(job.id)}">Avsluta uppföljning</button>` : ''}</div>
      </article>`).join('') : '<div class="settings-card"><p class="muted">Här visas bildjobben och dina sparade bilder.</p></div>';
    // Preserve focus and image loading when the poll contains no changes.
    if (target.dataset.rendered === html) return;
    target.dataset.rendered = html; target.innerHTML = html;
    this.bindPreviews(target);
    target.querySelectorAll<HTMLButtonElement>('[data-reuse-image]').forEach(button => button.onclick = () => {
      const job = jobs.find(j => j.id === button.dataset.reuseImage)!;
      if ((this.draft.prompt.trim() || this.draft.negative_prompt?.trim()) && !window.confirm('Ersätta bildutkastet med denna prompt?')) return;
      this.draft = { ...emptyDraft(), ...job.draft }; this.change(); this.render();
    });
    target.querySelectorAll<HTMLButtonElement>('[data-image-source]').forEach(button => button.onclick = () => {
      const job = jobs.find(j => j.id === button.dataset.imageSource)!;
      this.openRecording(job.draft.recording_id!, job.draft.run_id);
    });
    target.querySelectorAll<HTMLButtonElement>('[data-dismiss-image]').forEach(button => button.onclick = () => void (async () => {
      if (!window.confirm('Kontrollera först att ComfyUI inte kör bildjobbet. Uppföljningen avslutas och Whisper får starta igen. Detta avbryter inte jobbet i ComfyUI. Fortsätta?')) return;
      button.disabled = true;
      try { await invoke('dismiss_image_job', { id: button.dataset.dismissImage }); await this.refresh(); }
      catch (e) { this.notify(String(e), true); button.disabled = false; }
    })());
    target.querySelectorAll<HTMLButtonElement>('[data-follow-image]').forEach(button => button.onclick = () => void (async () => {
      button.disabled = true;
      try { await invoke('follow_image_job', { id: button.dataset.followImage }); await this.refresh(); }
      catch (e) { this.notify(String(e), true); button.disabled = false; }
    })());
  }

  private renderGallery() {
    if (!this.galleryContainer) return;
    if (!this.galleryContainer.querySelector('#all-images')) {
      this.galleryContainer.innerHTML = '<div class="page-heading"><div><div class="eyebrow">DITT BILDARKIV</div><h1>Galleri</h1><p>Alla dina skapade bilder, med de senaste först.</p></div></div><div id="all-images"></div>';
    }
    this.renderImageCards(this.galleryContainer.querySelector<HTMLElement>('#all-images')!);
  }
  private renderImageCards(target: HTMLElement, limit?: number) {
    const entries = galleryImages(this.snapshot?.jobs ?? []).slice(0, limit);
    const esc = this.esc;
    const html = entries.length ? `<div class="image-gallery">${entries.map(({ job, image }, index) => `<article class="settings-card gallery-card">
      ${this.thumbnail(image, job)}
      <p><b>${esc(job.config.workflow_name)}</b><br><small>${esc(new Date(image.created_at ?? job.created_at).toLocaleString('sv-SE'))}</small></p>
      <details><summary>Bildprompt</summary><p class="image-prompt-preview">${esc(job.draft.prompt)}</p>${job.draft.negative_prompt != null ? `<b>Negativ prompt</b><p class="image-prompt-preview">${esc(job.draft.negative_prompt)}</p>` : ''}</details>
      <button class="secondary small" data-delete="${index}">Ta bort bild</button></article>`).join('')}</div>` : '<p class="muted">Inga sparade bilder ännu.</p>';
    if (target.dataset.rendered === html) return;
    target.dataset.rendered = html; target.innerHTML = html;
    this.bindPreviews(target);
    target.querySelectorAll<HTMLButtonElement>('[data-delete]').forEach(button => button.onclick = () => void (async () => {
      if (!window.confirm('Ta bort bilden från ditt lokala bildarkiv? Bildfilen raderas. Originalet i ComfyUI påverkas inte.')) return;
      const { job, image } = entries[Number(button.dataset.delete)];
      button.disabled = true;
      try { await invoke('delete_generated_image', { id: job.id, path: image.path }); await this.refresh(); this.notify('Bilden har tagits bort.'); }
      catch (e) { this.notify(String(e), true); button.disabled = false; }
    })());
  }
  private thumbnail(image: GeneratedImage, job: ImageJob) {
    return `<button class="image-thumbnail" data-preview-path="${this.esc(image.path)}" aria-label="Visa bilden större"><img src="${this.esc(convertFileSrc(image.path))}" loading="lazy" alt="${this.esc(job.draft.prompt)}" /></button>`;
  }
  private bindPreviews(target: HTMLElement) {
    target.querySelectorAll<HTMLButtonElement>('[data-preview-path]').forEach(button => button.onclick = () => {
      const dialog = document.createElement('dialog'); dialog.className = 'image-preview-dialog';
      dialog.innerHTML = `<button class="secondary">Stäng</button><img src="${this.esc(convertFileSrc(button.dataset.previewPath!))}" alt="Skapad bild" />`;
      dialog.querySelector('button')!.onclick = () => dialog.close(); dialog.onclose = () => dialog.remove();
      document.body.append(dialog); dialog.showModal();
    });
  }
  private renderWorkflows() {
    const target = this.container.querySelector<HTMLElement>('#workflow-library'); if (!target) return;
    const workflows = this.snapshot?.workflows ?? [];
    const selected = this.snapshot?.settings.selected_workflow_id;
    const signature = JSON.stringify([workflows, selected]);
    if (target.dataset.rendered === signature) return;
    target.dataset.rendered = signature;
    target.innerHTML = `<div class="image-job-head"><h2>Mina flöden</h2><button id="add-workflow" class="secondary">Lägg till flöde</button></div><label>Flöde för nästa bild<select id="workflow-choice"><option value="">Välj ett sparat flöde…</option>${workflows.map(w => `<option value="${this.esc(w.id)}" ${w.id === selected ? 'selected' : ''}>${this.esc(w.name)}</option>`).join('')}</select></label><div class="device-actions"><button id="edit-workflow" class="text-button" ${selected ? '' : 'disabled'}>Ändra namn / promptfält</button></div><p class="muted">Varje flöde behåller modell, bildstorlek, negativa promptar och övriga egenskaper från sin API-JSON. Flöden sparas lokalt och finns kvar efter omstart.</p>`;
    target.querySelector<HTMLButtonElement>('#add-workflow')!.onclick = () => this.editWorkflow();
    target.querySelector<HTMLButtonElement>('#edit-workflow')!.onclick = () => this.editWorkflow(workflows.find(w => w.id === selected));
    target.querySelector<HTMLSelectElement>('#workflow-choice')!.onchange = event => void (async () => {
      const id = (event.target as HTMLSelectElement).value;
      if (!id || this.workflowChanging) { target.dataset.rendered = ''; this.renderWorkflows(); return; }
      this.workflowChanging = true; this.updateCreateButton();
      (event.target as HTMLSelectElement).disabled = true;
      try { await invoke('select_workflow', { id }); await this.refresh(); }
      catch (e) { this.notify(String(e), true); }
      finally { this.workflowChanging = false; target.dataset.rendered = ''; this.renderWorkflows(); this.updateCreateButton(); }
    })();
  }
  private editWorkflow(existing?: SavedWorkflow) {
    showWorkflowEditor(this.notify, async () => { await this.refresh(); }, existing);
  }
  showSettings() {
    const container = this.configContainer(); if (!container) return;
    const settings = this.snapshot?.settings;
    container.innerHTML = `<h2>ComfyUI – server</h2><p>Dina bildflöden importeras och väljs under Journal eller Bilder.</p><label>ComfyUI-adress<input id="comfy-url" value="${this.esc(settings?.url ?? 'http://127.0.0.1:8188')}" spellcheck="false" /></label><label>ComfyUI-mapp<input id="comfy-directory" value="${this.esc(settings?.comfy_directory)}" placeholder="/home/ditt-namn/comfy/ComfyUI" spellcheck="false" /></label><label>ComfyUI:s Pythonprogram<input id="comfy-python" value="${this.esc(settings?.comfy_python_path)}" placeholder="/home/ditt-namn/comfy/ComfyUI/.venv/bin/python" spellcheck="false" /></label><div class="device-actions"><button id="comfy-start" class="primary">Starta ComfyUI-server</button><button id="comfy-test" class="secondary">Testa anslutning</button><button id="comfy-save" class="secondary">Spara serverinställningar</button></div><p id="comfy-server-status" role="status" class="muted"></p><small>Servern startas lokalt utan att öppna webbläsaren. En server startad av DreamWhisper avslutas när du väljer Avsluta i systemfältet. En server du startat separat återanvänds.</small><div id="saved-workflow-settings"></div>`;
    const connection = () => ({ url: container.querySelector<HTMLInputElement>('#comfy-url')!.value.trim(), comfy_directory: container.querySelector<HTMLInputElement>('#comfy-directory')!.value.trim(), comfy_python_path: container.querySelector<HTMLInputElement>('#comfy-python')!.value.trim() });
    container.querySelector<HTMLButtonElement>('#comfy-save')!.onclick = () => void (async () => {
      try { await invoke('save_comfy_connection', { settings: connection() }); await this.refresh(); this.notify('Serverinställningar sparade.'); }
      catch (e) { this.notify(String(e), true); }
    })();
    container.querySelector<HTMLButtonElement>('#comfy-start')!.onclick = () => void (async () => {
      if (this.serverStarting) return;
      this.serverStarting = true; container.querySelector<HTMLButtonElement>('#comfy-start')!.disabled = true;
      try { await invoke('save_comfy_connection', { settings: connection() }); await invoke('start_comfy_server'); await this.refresh(); }
      catch (e) { this.notify(String(e), true); }
      finally { this.serverStarting = false; await this.refreshServerStatus(); }
    })();
    container.querySelector<HTMLButtonElement>('#comfy-test')!.onclick = () => void (async () => {
      const button = container.querySelector<HTMLButtonElement>('#comfy-test')!; button.disabled = true;
      try { await invoke('test_comfy_connection', { url: connection().url }); this.notify('Anslutningen till ComfyUI fungerar.'); }
      catch (e) { this.notify(String(e), true); } finally { button.disabled = false; }
    })();
    this.renderSavedWorkflowSettings();
    void this.refreshServerStatus();
  }
  private renderSavedWorkflowSettings() {
    const target = this.configContainer()?.querySelector<HTMLElement>('#saved-workflow-settings');
    if (!target) return;
    const workflows = this.snapshot?.workflows ?? [];
    const selected = this.snapshot?.settings.selected_workflow_id;
    const signature = JSON.stringify([workflows.map(w => [w.id, w.name]), selected]);
    if (target.dataset.rendered === signature) return;
    target.dataset.rendered = signature;
    target.innerHTML = `<h3>Sparade bildflöden</h3><p>Ta bort flöden du inte längre vill använda. Bildhistorik, köade jobb och sparade bilder behålls.</p>${workflows.length ? workflows.map(w => `<div class="workflow-settings-row"><div><b>${this.esc(w.name)}</b>${w.id === selected ? '<small>Valt för nästa bild</small>' : ''}</div><button type="button" class="secondary small" data-delete-workflow="${this.esc(w.id)}">Ta bort</button></div>`).join('') : '<p class="muted">Inga sparade flöden. Lägg till ett flöde under Journal eller Bilder.</p>'}`;
    target.querySelectorAll<HTMLButtonElement>('[data-delete-workflow]').forEach(button => button.onclick = () => void (async () => {
      const workflow = workflows.find(w => w.id === button.dataset.deleteWorkflow)!;
      if (!window.confirm(`Ta bort flödet ”${workflow.name}”? Tidigare bilder och bildjobb finns kvar.${workflow.id === selected ? ' Nästa sparade flöde väljs i stället.' : ''}`)) return;
      button.disabled = true;
      try { await invoke('delete_workflow', { id: workflow.id }); await this.refresh(); this.notify('Flödet har tagits bort.'); }
      catch (e) { this.notify(String(e), true); button.disabled = false; }
    })());
  }
  private async refreshServerStatus() {
    const container = this.configContainer();
    const text = container?.querySelector<HTMLElement>('#comfy-server-status');
    if (!text || !isTauri()) return;
    try {
      const server = await invoke<{ status: string; error: string | null; url: string; log_path: string }>('comfy_server_status');
      text.textContent = server.error ?? ({ starting: 'ComfyUI startar …', running: `ComfyUI kör på ${server.url}`, external: `En befintlig ComfyUI-server kör på ${server.url}`, stopped: 'Servern har avslutats.' } as Record<string, string>)[server.status] ?? 'Servern är inte startad av DreamWhisper.';
      if (server.log_path && !text.textContent.includes(server.log_path)) text.textContent += ` Logg: ${server.log_path}`;
      container!.querySelector<HTMLButtonElement>('#comfy-start')!.disabled = this.serverStarting || ['starting', 'running'].includes(server.status);
    } catch (e) { text.textContent = String(e); }
  }
}

export function showWorkflowEditor(notify: (text: string, error?: boolean) => void, onSaved: (id: string) => Promise<void>, existing?: SavedWorkflow) {
  let workflow = existing ? structuredClone(existing.workflow) : null;
  const dialog = document.createElement('dialog');
  dialog.className = 'workflow-dialog';
  dialog.innerHTML = `<form class="workflow-form"><h2>${existing ? 'Ändra sparat flöde' : 'Lägg till ComfyUI-flöde'}</h2><label>Namn<input id="workflow-name" maxlength="120" required value="${escapeHtml(existing?.name ?? '')}" placeholder="Exempel: Foto, illustration eller landskap" /></label><label>JSON i API-format<input id="workflow-file" type="file" accept=".json,application/json" ${existing ? '' : 'required'} /></label><small>Exportera i API-format från ComfyUI. ${existing ? 'Välj en ny fil om du vill ersätta detta flöde.' : ''}</small><label>Textfält för positiv prompt<select id="workflow-input" required></select></label><label>Textfält för negativ prompt (valfritt)<select id="workflow-negative-input"></select></label><div class="dialog-actions"><button type="button" class="secondary" id="workflow-cancel">Avbryt</button><button type="submit" class="primary">Spara flöde</button></div></form>`;
  const fields = () => {
    const select = dialog.querySelector<HTMLSelectElement>('#workflow-input')!;
    select.innerHTML = '<option value="">Välj promptfält…</option>' + (workflow ? workflowFields(workflow) : []).map(f => `<option value="${escapeHtml(JSON.stringify({ node: f.node, input: f.input }))}" ${existing && f.node === existing.node_id && f.input === existing.input_name ? 'selected' : ''}>${escapeHtml(f.title)}</option>`).join('');
    dialog.querySelector<HTMLSelectElement>('#workflow-negative-input')!.innerHTML = '<option value="">Ingen negativ prompt – behåll flödets inställningar</option>' + (workflow ? workflowFields(workflow) : []).map(f => `<option value="${escapeHtml(JSON.stringify({ node: f.node, input: f.input }))}" ${existing?.negative_field && f.node === existing.negative_field.node_id && f.input === existing.negative_field.input_name ? 'selected' : ''}>${escapeHtml(f.title)}</option>`).join('');
  };
  fields();
  dialog.querySelector<HTMLInputElement>('#workflow-file')!.onchange = event => void (async () => {
    const file = (event.target as HTMLInputElement).files?.[0]; if (!file) return;
    try {
      if (file.size > 5 * 1024 * 1024) throw new Error('Workflow-filen är större än 5 MB.');
      const imported = JSON.parse(await file.text());
      if (!workflowFields(imported).length) throw new Error('Workflow saknar redigerbara textfält.');
      workflow = imported;
      const name = dialog.querySelector<HTMLInputElement>('#workflow-name')!;
      if (!name.value.trim()) name.value = file.name.replace(/\.json$/i, '');
      fields();
    } catch (e) { workflow = existing ? structuredClone(existing.workflow) : null; fields(); notify(String(e), true); }
  })();
  dialog.querySelector('#workflow-cancel')!.addEventListener('click', () => dialog.close());
  dialog.onclose = () => dialog.remove();
  dialog.querySelector('form')!.onsubmit = event => { event.preventDefault(); void (async () => {
    const button = dialog.querySelector<HTMLButtonElement>('button[type=submit]')!; button.disabled = true;
    try {
      const field = JSON.parse(dialog.querySelector<HTMLSelectElement>('#workflow-input')!.value || 'null');
      const negative = JSON.parse(dialog.querySelector<HTMLSelectElement>('#workflow-negative-input')!.value || 'null');
      if (!workflow || !field) throw new Error('Importera API-JSON och välj positivt promptfält.');
      if (negative && negative.node === field.node && negative.input === field.input) throw new Error('Positiv och negativ prompt måste använda olika textfält.');
      const id = await invoke<string>('save_workflow', { workflow: { id: existing?.id ?? '', name: dialog.querySelector<HTMLInputElement>('#workflow-name')!.value.trim(), workflow, node_id: field.node, input_name: field.input, negative_field: negative ? { node_id: negative.node, input_name: negative.input } : null } });
      dialog.close(); await onSaved(id); notify('Flödet är sparat och valt för nästa bild.');
    } catch (e) { notify(String(e), true); button.disabled = false; }
  })(); };
  document.body.append(dialog); dialog.showModal();
}
