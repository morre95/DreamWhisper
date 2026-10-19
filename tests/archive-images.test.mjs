import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import ts from 'typescript';

const code = ts.transpileModule(readFileSync(new URL('../src/archive-images.ts', import.meta.url), 'utf8'), {
  compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext },
}).outputText;
const { ArchiveImages } = await import(`data:text/javascript;base64,${Buffer.from(code).toString('base64')}`);
globalThis.MutationObserver = undefined;
globalThis.document = undefined;
const settle = () => new Promise(resolve => setImmediate(resolve));

function setup(t, urls) {
  let changed;
  const previousObserver = globalThis.MutationObserver;
  const previousDocument = globalThis.document;
  globalThis.MutationObserver = class {
    constructor(callback) { changed = callback; }
    observe() {}
    disconnect() {}
  };
  globalThis.document = { createElement: () => ({
    setAttribute() {}, remove() { this.removed = true; }, textContent: '',
  }) };
  const images = urls.map(url => ({
    dataset: { archiveSrc: url }, src: '', hidden: false, error: null,
    removeAttribute(name) { if (name === 'src') this.src = ''; },
    after(error) { this.error = error; },
  }));
  const root = { images, querySelectorAll() { return this.images; } };
  const loader = new ArchiveImages(root);
  t.after(() => { loader.dispose(); globalThis.MutationObserver = previousObserver; globalThis.document = previousDocument; });
  return { loader, root, images, changed: () => changed() };
}

test('thumbnails and previews share a fetched Blob URL and release it after the last image is removed', async t => {
  const realFetch = globalThis.fetch;
  let fetches = 0;
  t.mock.method(globalThis, 'fetch', async () => {
    fetches++;
    return new Response(new Uint8Array([1, 2, 3]), { headers: { 'Content-Type': 'image/png' } });
  });
  const { root, images, changed } = setup(t, ['asset://picture', 'asset://picture']);
  await settle();
  const url = images[0].src;
  assert.ok(url.startsWith('blob:'));
  assert.equal(images[1].src, url);
  assert.equal(fetches, 1);
  assert.equal((await realFetch(url)).headers.get('Content-Type'), 'image/png');
  assert.deepEqual(new Uint8Array(await (await realFetch(url)).arrayBuffer()), new Uint8Array([1, 2, 3]));
  root.images = [images[1]]; changed();
  assert.equal((await realFetch(url)).status, 200);
  root.images = []; changed();
  await assert.rejects(realFetch(url));
});

test('removing an image aborts the request and prevents a late fetch from attaching a stale URL', async t => {
  let finish, signal;
  t.mock.method(globalThis, 'fetch', (_url, options) => {
    signal = options.signal;
    return new Promise(resolve => { finish = resolve; });
  });
  const { root, images, changed } = setup(t, ['asset://slow']);
  root.images = []; changed();
  assert.equal(signal.aborted, true);
  finish(new Response(new Uint8Array([1])));
  await settle();
  assert.equal(images[0].src, '');
  assert.equal(images[0].error, null);
});

test('a changed source reloads the image and releases the old Blob', async t => {
  const realFetch = globalThis.fetch;
  t.mock.method(globalThis, 'fetch', async () => new Response(new Uint8Array([1]), { headers: { 'Content-Type': 'image/png' } }));
  const { images, changed } = setup(t, ['asset://first']);
  await settle();
  const first = images[0].src;
  images[0].dataset.archiveSrc = 'asset://second'; changed();
  await settle();
  assert.notEqual(images[0].src, first);
  assert.ok(images[0].src.startsWith('blob:'));
  await assert.rejects(realFetch(first));
});

test('missing, denied, empty and undecodable images show a visible explanation', async t => {
  for (const [status, message] of [[403, /saknar åtkomst/], [404, /finns inte längre/], [500, /fel 500/], [200, /är tom/]]) {
    t.mock.method(globalThis, 'fetch', async () => new Response('', { status }));
    const { loader, images } = setup(t, ['asset://missing']);
    await settle();
    assert.equal(images[0].hidden, true);
    assert.match(images[0].error.textContent, message);
    loader.dispose();
  }
  t.mock.method(globalThis, 'fetch', async () => new Response(new Uint8Array([1]), { headers: { 'Content-Type': 'image/png' } }));
  const { images } = setup(t, ['asset://invalid']);
  await settle(); images[0].onerror();
  assert.match(images[0].error.textContent, /kunde inte visas/);
});

test('all image views use the Blob loader and both CSP modes permit it', () => {
  for (const file of ['images.ts', 'journal.ts']) {
    const source = readFileSync(new URL(`../src/${file}`, import.meta.url), 'utf8');
    const tags = source.match(/<img\b[^>]+>/g);
    assert.equal(tags.length, 2, `${file}: thumbnail and preview`);
    for (const tag of tags) {
      assert.match(tag, /data-archive-src=/);
      assert.doesNotMatch(tag, /\s src=/);
    }
  }
  const { app } = JSON.parse(readFileSync(new URL('../src-tauri/tauri.conf.json', import.meta.url), 'utf8'));
  for (const csp of [app.security.csp, app.security.devCsp]) {
    assert.match(csp.match(/img-src[^;]+/)[0], /\bblob:/);
  }
});
