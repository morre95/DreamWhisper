/** Load through Tauri's scoped asset fetch, then give WebKit a regular media Blob.
 * Direct asset:// media URLs fail on Linux even when the MP3 decoder is available.
 */
export class ArchiveAudio {
  readonly ready: Promise<void>;
  private readonly controller = new AbortController();
  private objectUrl: string | null = null;

  constructor(private readonly audio: HTMLAudioElement, url: string, mimeType: string) {
    this.ready = this.load(url, mimeType);
  }

  private async load(url: string, mimeType: string): Promise<void> {
    const response = await fetch(url, { signal: this.controller.signal });
    if (!response.ok) {
      if (response.status === 403) throw new Error('Appen saknar åtkomst till arkivfilen.');
      if (response.status === 404) throw new Error('Arkivfilen finns inte längre på datorn.');
      throw new Error(`Ljudfilen kunde inte läsas (fel ${response.status}).`);
    }
    const blob = await response.blob();
    // A previous selection may have finished fetching after this player was disposed.
    if (this.controller.signal.aborted) return;
    if (blob.size === 0) throw new Error('Arkivfilen är tom.');
    this.objectUrl = URL.createObjectURL(new Blob([blob], { type: mimeType }));
    this.audio.src = this.objectUrl;
    this.audio.load();
  }

  async playFrom(seconds: number): Promise<void> {
    await this.ready;
    if (this.controller.signal.aborted) return;
    this.audio.currentTime = seconds;
    await this.audio.play();
  }

  dispose(): void {
    this.controller.abort();
    this.audio.pause();
    this.audio.removeAttribute('src');
    this.audio.load();
    if (this.objectUrl) {
      URL.revokeObjectURL(this.objectUrl);
      this.objectUrl = null;
    }
  }
}
