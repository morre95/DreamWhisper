import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import ts from 'typescript';

let handle;
globalThis.__journalTestBridge = { invoke: (...args) => handle(...args), convertFileSrc: path => path, isTauri: () => true };
const source = readFileSync(new URL('../src/journal.ts', import.meta.url), 'utf8')
  .replace(/import .* from '@tauri-apps\/api\/core';/, 'const { invoke, convertFileSrc, isTauri } = globalThis.__journalTestBridge;')
  .replace(/import .* from '\.\/images';/, 'const esc = value => String(value); const showWorkflowEditor = () => {};');
const code = ts.transpileModule(source, { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext } }).outputText;
const { JournalView, composeScene, latestDrafts } = await import(`data:text/javascript;base64,${Buffer.from(code).toString('base64')}`);

const entry = () => ({ id: 'entry', version: 1, title: 'Dream', date: '2026-10-06', kind: 'dream', language: 'sv', description: 'Original', reflections: 'Private', description_revision: 'revision', confirmed_revision: 'revision', recording_id: null, run_id: null, favourite_image: null });
const scene = () => ({ title: 'Forest', summary: 'Scene', details: [{ text: 'Forest', quote: 'forest', english: 'A forest' }], additions: [{ text: 'Light', quote: null, english: 'Soft light' }], questions: [] });
const draft = (id, parent = null) => ({ id, parent_id: parent, entry_id: 'entry', description_revision: 'revision', scene: scene(), prompt: 'A forest. Soft light', negative_prompt: null, arrangement: '', manual: false, approved: false });

test('scene review preserves source order across edits and newest drafting batches', () => {
  const values = [{ ...draft('old-second'), position: 1, batch_created_at: '2026-10-06T10:00:00Z' }, { ...draft('old-first-edited','old-first'), position: 0, batch_created_at: '2026-10-06T10:00:00Z' }, { ...draft('old-first'), position: 0, batch_created_at: '2026-10-06T10:00:00Z' }, { ...draft('new-first'), position: 0, batch_created_at: '2026-10-06T11:00:00Z' }];
  assert.deepEqual(latestDrafts(values).map(d => d.id), ['new-first','old-first-edited','old-second']);
});

test('removing a creative addition removes it from the generated prompt', () => {
  const value = scene(); assert.equal(composeScene(value), 'A forest. Soft light');
  value.additions = []; assert.equal(composeScene(value), 'A forest');
});

test('review shows the latest revision of each scene without losing independent scenes', () => {
  assert.deepEqual(latestDrafts([draft('three','two'), draft('two','one'), draft('one'), draft('other')]).map(d => d.id), ['three','other']);
});

function editor() {
  const state = { entry: entry(), drafts: [], jobs: [], descriptions: [], settings: {}, setup_ready: false };
  const values = { title: 'Dream', date: '2026-10-06', kind: 'dream', language: 'sv', description: 'Changed', reflections: 'Private' };
  const form = { values };
  const status = { textContent: '' }; const confirm = { textContent: '' }; const button = { disabled: false };
  const container = { querySelector: selector => ({ '#journal-form': form, '#journal-save-status': status, '#confirmation-status': confirm, '#draft-scenes': button })[selector] ?? null };
  const view = new JournalView(container, () => {}, () => {});
  view.entry = state.entry; view.entryDirty = true; view.editSequence = 1;
  view.snapshot = { ...state, entries: [state.entry] };
  const original = globalThis.FormData;
  globalThis.FormData = class { constructor(form) { this.data = { ...form.values }; } get(key) { return this.data[key]; } };
  return { view, state, values, status, restore: () => { globalThis.FormData = original; } };
}

test('autosaves serialize revisions and persist edits made during an in-flight save', async () => {
  const { view, state, values, restore } = editor(); const writes = []; let finish;
  handle = async (command, args) => {
    if (command === 'journal_snapshot') return { ...state, entries: [state.entry] };
    if (command === 'image_snapshot') return { jobs: [], workflows: [], settings: {} };
    assert.equal(command, 'save_journal_entry'); writes.push(args.input);
    if (writes.length === 1) await new Promise(resolve => { finish = resolve; });
    state.entry = { ...state.entry, ...args.input, version: state.entry.version + 1 }; return state.entry;
  };
  try {
    const first = view.flush();
    while (!finish) await new Promise(resolve => setImmediate(resolve));
    values.description = 'Newest'; view.editSequence++; view.entryDirty = true;
    const second = view.flush(); finish(); await Promise.all([first, second]);
    assert.deepEqual(writes.map(w => [w.version,w.description]), [[1,'Changed'],[2,'Newest']]);
    assert.equal(view.entry.description, 'Newest'); assert.equal(view.dirty, false);
  } finally { restore(); }
});

test('a failed save keeps edits available for a subsequent retry', async () => {
  const { view, state, restore } = editor(); let fail = true;
  handle = async (command, args) => {
    if (command === 'journal_snapshot') return { ...state, entries: [state.entry] };
    if (command === 'image_snapshot') return { jobs: [], workflows: [], settings: {} };
    if (fail) throw new Error('disk full');
    state.entry = { ...state.entry, ...args.input, version: 2 }; return state.entry;
  };
  try { await assert.rejects(view.flush(), /disk full/); assert.equal(view.dirty, true); fail = false; await view.flush(); assert.equal(view.dirty, false); }
  finally { restore(); }
});

test('scene edits during a save become a child of the saved revision and keep approval cleared', async () => {
  const { view, state, restore } = editor(); view.entryDirty = false;
  const edit = { id: 'old', scene: scene(), prompt: 'First', manual: true, negative_prompt: null, arrangement: '' };
  view.edits.set('old', edit); view.selected.add('old'); const writes = []; let finish;
  handle = async (command, args) => {
    if (command === 'journal_snapshot') return { ...state, entries: [state.entry] };
    if (command === 'image_snapshot') return { jobs: [], workflows: [], settings: {} };
    assert.equal(command,'save_scene_draft'); writes.push(args.edit);
    if (writes.length === 1) await new Promise(resolve => { finish = resolve; });
    const saved = { ...draft(`new-${writes.length}`,args.edit.id), ...args.edit, id: `new-${writes.length}`, approved: false };
    state.drafts.unshift(saved); return saved;
  };
  try {
    const save = view.flush(); while (!finish) await new Promise(resolve => setImmediate(resolve));
    edit.prompt = 'Newest'; finish(); await save;
    assert.deepEqual(writes.map(w => [w.id,w.prompt]), [['old','First'],['new-1','Newest']]);
    assert.deepEqual([...view.selected], ['new-2']); assert.equal(view.dirty,false);
  } finally { restore(); }
});


test('imported workflows become selectable while an unsaved scene prompt stays intact', async () => {
  const select = { dataset: {}, innerHTML: '' };
  const hint = { textContent: '' };
  const results = { querySelector: () => null };
  const container = { querySelector: selector => ({ '#journal-workflow': select, '#journal-workflow-hint': hint, '#journal-results': results })[selector] ?? null };
  const view = new JournalView(container, () => {}, () => {});
  view.visible = true; view.entry = entry();
  const edit = { prompt: 'My unfinished scene changes' };
  view.edits.set('scene', edit);
  view.renderList = () => {};
  view.updateButtons = () => {};
  view.workflowId = 'imported';
  handle = async command => {
    if (command === 'journal_snapshot') return { entries: [entry()], drafts: [], jobs: [], descriptions: [] };
    assert.equal(command, 'image_snapshot');
    return { jobs: [], workflows: [{ id: 'old', name: 'Existing' }, { id: 'imported', name: 'New workflow' }], settings: { selected_workflow_id: 'imported' } };
  };
  await view.refresh();
  assert.match(select.innerHTML, /value="imported" selected>New workflow/);
  assert.equal(view.edits.get('scene'), edit);
  assert.equal(edit.prompt, 'My unfinished scene changes');
  // A later global selection change must preserve the workflow chosen in Journal.
  view.workflowId = 'old';
  await view.refresh();
  assert.match(select.innerHTML, /value="old" selected>Existing/);
});
