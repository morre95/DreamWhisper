import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import ts from 'typescript';

let handle;
globalThis.__imageTestBridge = { invoke: (...args) => handle(...args), convertFileSrc: path => path, isTauri: () => true };
globalThis.window = { confirm: () => true };
const seedSource = readFileSync(new URL('../src/image-seed.ts', import.meta.url), 'utf8');
const seedCode = ts.transpileModule(seedSource, { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext } }).outputText;
const seedUrl = `data:text/javascript;base64,${Buffer.from(seedCode).toString('base64')}`;
const { SeedControl, seedError, randomSeed } = await import(seedUrl);
const source = readFileSync(new URL('../src/images.ts', import.meta.url), 'utf8')
  .replace("from './image-seed';", `from '${seedUrl}';`)
  .replace(/import .* from '@tauri-apps\/api\/core';/, 'const { invoke, convertFileSrc, isTauri } = (globalThis as any).__imageTestBridge;');
const code = ts.transpileModule(source, { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext } }).outputText;
const { ImagesView, selectedText, workflowFields, escapeHtml, galleryImages } = await import(`data:text/javascript;base64,${Buffer.from(code).toString('base64')}`);
const snapshot = (prompt = '') => ({ settings: { url: 'http://127.0.0.1:8188', workflow: {}, workflow_name: '', node_id: '', input_name: '' }, draft: { prompt, source_text: '', recording_id: null, run_id: null }, jobs: [] });
const view = () => new ImagesView({}, () => null, () => {}, () => {});

test('seed controls keep 64-bit values exact, randomize on request, reset, and flag unsupported flows', () => {
  const workflow = { '3': { inputs: { seed: 123 } } };
  const input = { value: '', setCustomValidity(value) { this.error = value; }, setAttribute(name, value) { this[name] = value; } };
  const random = {}, reset = {}, hint = {};
  const container = { querySelector: selector => ({ '#seed': input, '#seed-random': random, '#seed-reset': reset, '#seed-hint': hint })[selector] };
  let value = '';
  const control = new SeedControl(container, 'seed', next => { value = next; control.update(workflow, value); });
  control.update(workflow, value);
  input.value = '18446744073709551615'; input.oninput();
  assert.equal(value, '18446744073709551615'); assert.equal(input.error, '');
  random.onclick();
  assert.notEqual(value, '18446744073709551615'); assert.equal(seedError(value, workflow), '');
  const chosen = value; control.update(workflow, value); assert.equal(value, chosen);
  reset.onclick(); assert.equal(value, ''); assert.equal(input.value, '');
  input.value = '-1'; input.oninput(); assert.match(input.error, /heltal/);
  input.value = '0'; input.oninput(); assert.equal(input.error, '');
  control.update({ '3': { inputs: { seed: ['4', 0] } } }, '0');
  assert.equal(random.disabled, true); assert.equal(reset.disabled, false); assert.match(hint.textContent, /saknar/);
  for (const invalid of ['1.5', '1e3', '+1', '18446744073709551616']) assert.match(seedError(invalid, workflow), /heltal/);
  assert.equal(seedError('42', { '3': { inputs: { noise_seed: 0 } } }), '');
});

test('randomization avoids returning the previous seed even when random bytes repeat', () => {
  const original = Object.getOwnPropertyDescriptor(globalThis, 'crypto');
  Object.defineProperty(globalThis, 'crypto', { configurable: true, value: { getRandomValues: array => array.fill(0xffffffff) } });
  try { assert.equal(randomSeed('18446744073709551615'), '0'); }
  finally { if (original) Object.defineProperty(globalThis, 'crypto', original); else delete globalThis.crypto; }
});

test('image generation persists and sends the selected seed as text without changing the prompt', async () => {
  const state = snapshot('Same prompt');
  state.workflows = [{ id: 'flow', workflow: { '3': { inputs: { seed: 123 } } } }];
  state.settings.selected_workflow_id = 'flow';
  const button = {}; const writes = [];
  const editor = new ImagesView({ querySelector: selector => selector === '#generate-image' ? button : null }, () => null, () => {}, () => {});
  handle = async (command, args) => {
    if (command === 'image_snapshot') return state;
    writes.push([command, args]);
  };
  await editor.refresh();
  editor.draft.seed = '18446744073709551615'; editor.change();
  await editor.create();
  assert.equal(writes[0][0], 'save_image_draft');
  assert.equal(writes[0][1].draft.seed, '18446744073709551615');
  assert.equal(writes[1][0], 'create_image');
  assert.equal(writes[1][1].draft.seed, '18446744073709551615');
  assert.equal(writes[1][1].draft.prompt, 'Same prompt');
  assert.equal(button.disabled, false);
  editor.draft.seed = '-1'; editor.updateCreateButton(); assert.equal(button.disabled, true);
});

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
  assert.deepEqual(writes[0], { prompt: 'Rättad text med detaljer', negative_prompt: null, seed: null, source_text: 'Rättad text med detaljer', recording_id: 'recording', run_id: 'version' });
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
