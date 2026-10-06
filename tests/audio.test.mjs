import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import ts from 'typescript';

const code = ts.transpileModule(readFileSync(new URL('../src/audio.ts', import.meta.url), 'utf8'), {
  compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext },
}).outputText;
const { ArchiveAudio } = await import(`data:text/javascript;base64,${Buffer.from(code).toString('base64')}`);

function player() {
  return {
    src: '', currentTime: 0, paused: false, loads: 0, plays: 0,
    load() { this.loads++; }, pause() { this.paused = true; },
    removeAttribute(name) { if (name === 'src') this.src = ''; },
    async play() { this.plays++; },
  };
}

test('loads a typed blob, seeks, and releases the URL on disposal', async t => {
  const realFetch = globalThis.fetch;
  t.mock.method(globalThis, 'fetch', async (url, options) => {
    if (url === 'asset://recording') {
      assert.ok(options.signal instanceof AbortSignal);
      return new Response(new Uint8Array([1, 2, 3]), { headers: { 'Content-Type': 'application/octet-stream' } });
    }
    return realFetch(url, options);
  });
  const audio = player();
  const source = new ArchiveAudio(audio, 'asset://recording', 'audio/mpeg');
  await source.ready;
  const blobUrl = audio.src;
  assert.ok(blobUrl.startsWith('blob:'));
  const response = await realFetch(blobUrl);
  assert.equal(response.headers.get('Content-Type'), 'audio/mpeg');
  assert.deepEqual(new Uint8Array(await response.arrayBuffer()), new Uint8Array([1, 2, 3]));
  await source.playFrom(4.5);
  assert.equal(audio.currentTime, 4.5);
  assert.equal(audio.plays, 1);
  source.dispose();
  assert.equal(audio.src, '');
  assert.equal(audio.paused, true);
  await assert.rejects(realFetch(blobUrl));
});

test('does not attach a stale fetch after the user switches recordings', async t => {
  let finish;
  t.mock.method(globalThis, 'fetch', () => new Promise(resolve => { finish = resolve; }));
  const audio = player();
  const source = new ArchiveAudio(audio, 'asset://slow', 'audio/wav');
  source.dispose();
  finish(new Response(new Uint8Array([1, 2, 3])));
  await source.ready;
  assert.equal(audio.src, '');
  await source.playFrom(5);
  assert.equal(audio.plays, 0);
});

test('distinguishes denied access from a missing archive file', async t => {
  for (const [status, message] of [[403, /saknar åtkomst/], [404, /finns inte längre/]]) {
    t.mock.method(globalThis, 'fetch', async () => new Response('', { status }));
    const audio = player();
    const source = new ArchiveAudio(audio, 'asset://missing', 'audio/mpeg');
    await assert.rejects(source.ready, message);
    assert.equal(audio.src, '');
    source.dispose();
    t.mock.restoreAll();
  }
});

test('rejects an empty archive instead of reporting an unsupported codec', async t => {
  t.mock.method(globalThis, 'fetch', async () => new Response(new Blob([])));
  const audio = player();
  const source = new ArchiveAudio(audio, 'asset://empty', 'audio/mpeg');
  await assert.rejects(source.ready, /Arkivfilen är tom/);
  source.dispose();
});
