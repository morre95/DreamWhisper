/** WebKitGTK can fetch asset:// images but fails to display them directly.
 * Share a Blob URL between thumbnails and previews, releasing it when unused.
 */
interface ImageSource {
  controller: AbortController;
  ready: Promise<string | null>;
  url: string | null;
  users: number;
}
interface ImageBinding { source: string; error?: HTMLElement }

export class ArchiveImages {
  private readonly sources = new Map<string, ImageSource>();
  private readonly images = new Map<HTMLImageElement, ImageBinding>();
  private readonly observer: MutationObserver;

  constructor(private readonly root: ParentNode) {
    this.observer = new MutationObserver(() => this.sync());
    this.observer.observe(root, { childList: true, subtree: true, attributes: true, attributeFilter: ['data-archive-src'] });
    this.sync();
  }

  sync(): void {
    const current = new Set(this.root.querySelectorAll<HTMLImageElement>('img[data-archive-src]'));
    for (const [image, binding] of this.images) {
      if (current.has(image) && image.dataset.archiveSrc === binding.source) continue;
      binding.error?.remove();
      image.onerror = null;
      image.removeAttribute('src');
      this.images.delete(image);
      const source = this.sources.get(binding.source)!;
      if (--source.users === 0) {
        source.controller.abort();
        if (source.url) URL.revokeObjectURL(source.url);
        this.sources.delete(binding.source);
      }
    }
    for (const image of current) {
      const url = image.dataset.archiveSrc;
      if (!url || this.images.has(image)) continue;
      const binding: ImageBinding = { source: url };
      this.images.set(image, binding);
      image.hidden = false;
      let source = this.sources.get(url);
      if (!source) {
        source = { controller: new AbortController(), ready: Promise.resolve(null), url: null, users: 0 };
        source.ready = this.load(url, source);
        this.sources.set(url, source);
      }
      source.users++;
      void source.ready.then(blobUrl => {
        if (!blobUrl || this.images.get(image) !== binding) return;
        image.onerror = () => this.fail(image, binding, 'Bildfilen kunde inte visas.');
        image.src = blobUrl;
      }).catch(error => {
        if (this.images.get(image) === binding) this.fail(image, binding, String(error instanceof Error ? error.message : error));
      });
    }
  }

  private async load(url: string, source: ImageSource): Promise<string | null> {
    const response = await fetch(url, { signal: source.controller.signal });
    if (!response.ok) {
      if (response.status === 403) throw new Error('Appen saknar åtkomst till bildfilen.');
      if (response.status === 404) throw new Error('Bildfilen finns inte längre på datorn.');
      throw new Error(`Bildfilen kunde inte läsas (fel ${response.status}).`);
    }
    const blob = await response.blob();
    if (source.controller.signal.aborted) return null;
    if (!blob.size) throw new Error('Bildfilen är tom.');
    source.url = URL.createObjectURL(blob);
    return source.url;
  }

  private fail(image: HTMLImageElement, binding: ImageBinding, message: string): void {
    if (binding.error) return;
    const error = document.createElement('span');
    error.className = 'inline-error';
    error.setAttribute('role', 'status');
    error.textContent = message;
    image.hidden = true;
    image.after(error);
    binding.error = error;
  }

  dispose(): void {
    this.observer.disconnect();
    for (const [image, binding] of this.images) {
      image.onerror = null;
      image.removeAttribute('src');
      binding.error?.remove();
    }
    for (const source of this.sources.values()) {
      source.controller.abort();
      if (source.url) URL.revokeObjectURL(source.url);
    }
    this.images.clear();
    this.sources.clear();
  }
}
