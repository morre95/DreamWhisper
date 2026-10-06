import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import ts from 'typescript';

let handle;
globalThis.__imageTestBridge = { invoke: (...args) => handle(...args), convertFileSrc: path => path, isTauri: () => true };
globalThis.window = { confirm: () => true };
const source = readFileSync(new URL('../src/images.ts', import.meta.url), 'utf8')
  .replace(/import .* from '@tauri-apps\/api\/core';/, 'const { invoke, convertFileSrc, isTauri } = (globalThis as any).__imageTestBridge;');
const code = ts.transpileModule(source, { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext } }).outputText;
const { ImagesView, selectedText, workflowFields, escapeHtml, galleryImages } = await import(`data:text/javascript;base64,${Buffer.from(code).toString('base64')}`);
const snapshot = (prompt = '') => ({ settings: { url: 'http://127.0.0.1:8188', workflow: {}, workflow_name: '', node_id: '', input_name: '' }, draft: { prompt, source_text: '', recording_id: null, run_id: null }, jobs: [] });
const view = () => new ImagesView({}, () => null, () => {}, () => {});

test('lists explicit text inputs without selecting a positive or negative node automatically', () => {
  const workflow = {
    '6': { class_type: 'CLIPTextEncode', inputs: { text: 'positive', clip: ['4', 1] }, _meta: { title: 'Positiv prompt' } },
    '7': { class_type: 'CLIPTextEncode', inputs: { text: 'negative' } },
  };
  assert.deepEqual(workflowFields(workflow), [
    { node: '6', input: 'text', title: 'Positiv prompt · nod 6 · text' },
    { node: '7', input: 'text', title: 'CLIPTextEncode · nod 7 · text' },
  ]);
  assert.equal(workflow['6'].inputs.text, 'positive');
  assert.throws(() => workflowFields({ nodes: [], links: [] }), /API/);
});

test('captures selections across paragraphs but rejects selections outside the transcript', () => {
  const first = {}, last = {}, outside = {};
  const container = { contains: node => node === first || node === last };
  const selection = { isCollapsed: false, rangeCount: 1, getRangeAt: () => ({ startContainer: first, endContainer: last }), toString: () => '  Första stycket.\n\nAndra stycket.  ' };
  assert.equal(selectedText(container, selection), 'Första stycket.\n\nAndra stycket.');
  assert.equal(selectedText(container, { ...selection, getRangeAt: () => ({ startContainer: first, endContainer: outside }) }), '');
  assert.equal(selectedText(container, { ...selection, isCollapsed: true }), '');
  assert.equal(selectedText(container, null), '');
});

test('selected corrected text is persisted as an independent draft with its source version', async () => {
  const writes = [];
  handle = async (command, args) => {
    if (command === 'image_snapshot') return snapshot();
    assert.equal(command, 'save_image_draft'); writes.push(args.draft);
  };
  const editor = view(); await editor.refresh();
  assert.equal(await editor.fromSelection('Rättad text med detaljer', 'recording', 'version'), true);
  assert.deepEqual(writes[0], { prompt: 'Rättad text med detaljer', negative_prompt: null, source_text: 'Rättad text med detaljer', recording_id: 'recording', run_id: 'version' });
  // No save_segment call, and a later polling snapshot cannot overwrite the draft.
  await editor.refresh(); await editor.persist(); assert.equal(writes.length, 1);
});

test('declining overwrite preserves the existing prompt', async () => {
  const writes = [];
  handle = async (command, args) => command === 'image_snapshot' ? snapshot('Existing prompt') : writes.push(args);
  window.confirm = () => false;
  try {
    const editor = view(); await editor.refresh();
    assert.equal(await editor.fromSelection('replacement', 'id', 'run'), false);
    assert.equal(writes.length, 0);
  } finally { window.confirm = () => true; }
});

test('overlapping draft saves serialize and persist the latest edit last', async () => {
  const writes = []; let finish;
  handle = async (command, args) => {
    if (command === 'image_snapshot') return snapshot();
    writes.push(args.draft.prompt);
    if (writes.length === 1) await new Promise(resolve => { finish = resolve; });
  };
  const editor = view(); await editor.refresh();
  const first = editor.fromSelection('first', 'id', 'run');
  const second = editor.fromSelection('second', 'id', 'run');
  finish(); await Promise.all([first, second]);
  assert.deepEqual(writes, ['first', 'second']);
});

test('prompt and workflow titles are escaped before displaying them', () => {
  assert.equal(escapeHtml('<script>"&'), '&lt;script&gt;&quot;&amp;');
});


test('gallery filters deleted images and limits individual images rather than jobs', () => {
  const job = { id: 'batch', created_at: '2026-01-01', images: Array.from({ length: 8 }, (_, i) => ({ path: String(i), deleted: i === 7 })) };
  const laterImage = { id: 'old-job', created_at: '2025-01-01', images: [{ path: 'latest', created_at: '2026-02-01' }] };
  const entries = galleryImages([job, laterImage]);
  assert.equal(entries.length, 8);
  assert.deepEqual(entries.slice(0, 5).map(e => e.image.path), ['latest', '6', '5', '4', '3']);
  assert.equal(galleryImages([]).length, 0);
});
