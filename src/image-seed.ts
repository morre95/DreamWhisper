type Workflow = Record<string, { inputs: Record<string, unknown> }>;
const maxSeed = 18446744073709551615n;

export function seedFields(workflow?: Workflow) {
  return Object.values(workflow ?? {}).flatMap(node => Object.entries(node.inputs)
    .filter(([name, value]) => ['seed', 'noise_seed'].includes(name) && typeof value === 'number' && Number.isInteger(value) && value >= 0));
}

export function seedError(value: string, workflow?: Workflow): string {
  const seed = value.trim();
  if (!seed) return '';
  if (!/^\d{1,20}$/.test(seed) || BigInt(seed) > maxSeed) return 'Ange ett heltal mellan 0 och 18446744073709551615.';
  return seedFields(workflow).length ? '' : 'Flödet saknar ett direkt numeriskt seed- eller noise_seed-fält. Töm seedfältet för att använda flödets inställningar.';
}

export function randomSeed(previous: string): string {
  const words = crypto.getRandomValues(new Uint32Array(2));
  let value = (BigInt(words[0]) << 32n) | BigInt(words[1]);
  if (value.toString() === previous.trim()) value = (value + 1n) & maxSeed;
  return value.toString();
}

export function seedControlMarkup(id: string) {
  return `<div class="seed-control"><label for="${id}">Seed (valfritt)</label><div class="seed-actions"><input id="${id}" type="text" inputmode="numeric" maxlength="20" placeholder="Använd flödets seed" autocomplete="off" spellcheck="false" aria-describedby="${id}-hint" /><button id="${id}-random" type="button" class="secondary">Slumpa seed</button><button id="${id}-reset" type="button" class="text-button">Använd flödets seed</button></div><small id="${id}-hint" class="muted" aria-live="polite"></small></div>`;
}

export class SeedControl {
  private input: HTMLInputElement;
  private random: HTMLButtonElement;
  private reset: HTMLButtonElement;
  private hint: HTMLElement;
  constructor(container: HTMLElement, id: string, onChange: (value: string) => void) {
    this.input = container.querySelector<HTMLInputElement>(`#${id}`)!;
    this.random = container.querySelector<HTMLButtonElement>(`#${id}-random`)!;
    this.reset = container.querySelector<HTMLButtonElement>(`#${id}-reset`)!;
    this.hint = container.querySelector<HTMLElement>(`#${id}-hint`)!;
    this.input.oninput = () => onChange(this.input.value);
    this.random.onclick = () => onChange(randomSeed(this.input.value));
    this.reset.onclick = () => onChange('');
  }
  update(workflow: Workflow | undefined, value: string) {
    const fields = seedFields(workflow);
    const error = seedError(value, workflow);
    if (this.input.value !== value) this.input.value = value;
    this.input.disabled = this.random.disabled = !fields.length;
    this.reset.disabled = !value;
    this.input.setCustomValidity(error);
    this.input.setAttribute('aria-invalid', String(!!error));
    this.hint.textContent = error || (!fields.length ? 'Flödet saknar ett direkt numeriskt seedfält. Seed via nodkopplingar ändras i ComfyUI.' : `Tomt fält behåller flödets seed. Slumpa seed och välj sedan Skapa bild / Generera bild för en ny variant med samma prompt.${fields.length > 1 ? ` Ditt seed används i alla ${fields.length} numeriska seedfält.` : ''}`);
  }
}
