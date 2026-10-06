# DreamWhisper

En personlig Linuxjournal som skapar bilder av beskrivna drömmar och meditationer. Skriv svenska eller engelska, eller importera Sony ICD-UX570-inspelningar. Original, transkript, reflektioner, scenutkast och bilder sparas lokalt.

Licens: [MIT](LICENSE). Copyright © 2026 Erik Morén.

## Starta appen

Kräver Rust, Node.js 22.12+ och Linuxbiblioteken för Tauri 2: GTK 3, WebKitGTK 4.1 och Ayatana AppIndicator. UDisks2 ska vara igång på systemets D-Bus. Projektets npm- och Cargo-lockfiler låser desktopberoendena.

```bash
npm ci
npm run desktop
```

Appen fungerar som ljudarkiv innan transkribering konfigurerats. `npm run dev` öppnar enbart en webbförhandsvisning; USB, import och SQLite kräver Tauriappen.

På NVIDIA med Wayland aktiverar appen automatiskt `WEBKIT_DISABLE_DMABUF_RENDERER=1` innan GTK/WebKit startas. Detta kringgår det dokumenterade [WebKitGTK-felet med Gdk Error 71](https://v2.tauri.app/develop/debug/linux-graphics/), med en långsammare renderingsväg för gränssnittet. CUDA-transkriberingen påverkas inte. En explicit inställning av variabeln respekteras, exempelvis `WEBKIT_DISABLE_DMABUF_RENDERER=0 npm run desktop` för att prova standardrenderingen efter en drivrutinsuppdatering.

När fönstret stängs ligger appen kvar i systemfältet och importerar i bakgrunden. Välj **Öppna DreamWhisper** eller **Avsluta** i ikonens meny. Kör en enda instans per arkiv. I en byggd app kan du aktivera **Starta i systemfältet vid inloggning** under Inställningar. Det skapar appens egen XDG-autostartfil och startar med `--background`. Funktionen kräver att skrivbordsmiljön följer XDG-autostart; den aktiveras inte av implementationen eller installationen. Utvecklingsläget med Vite kan inte användas för autostart.

## Installera transkribering

Använd Python 3.12 och en fungerande NVIDIA-drivrutin. CUDA 12 och cuDNN 9-biblioteken installeras i Pythonmiljön av skriptet. NVIDIA-drivrutinen installeras inte av appen. Kontrollera att `nvidia-smi` fungerar på datorn först.

```bash
bash scripts/setup-worker.sh
```

Detta hämtar Pythonberoenden, inklusive stora NVIDIA-bibliotek. Hämta sedan modellen en gång till en lokal mapp, exempelvis i projektet:

```bash
.venv/bin/python worker/prepare_model.py --output "$PWD/models/kb-whisper-large"
```

Modellhämtningen kräver internet och flera GB ledigt utrymme. Skriptet slår upp `KBLab/kb-whisper-large`, låser hämtningen till ett exakt modellcommit och skriver `dreamwhisper-model.json`. Du kan ange `--revision COMMIT` för en viss revision. Välj en separat mapp vid modellbyte; skriv inte över en modell som används.

Öppna **Inställningar** i appen och ange:

- Python: den absoluta sökvägen till projektets `.venv/bin/python`.
- Modellmapp: den absoluta sökvägen till den hämtade modellmappen.
- Batchstorlek: 8.
- Aktivera transkribering och spara.

I utvecklingsläge fylls sökvägarna automatiskt om projektets `.venv` och `models/kb-whisper-large` finns. Transkribering är fortfarande avstängd tills du aktiverar den.

Workern kör offline och laddar inte ner modeller. Välj svenska eller engelska på varje inspelning; svenska är standard. Den håller en modell laddad och kör ett jobb åt gången med det valda språket, CUDA, FP16, VAD och ordtidsstämplar. Vid GPU-minnesbrist halveras batchstorleken ner till 1. Om en fil fortfarande misslyckas behålls den i arkivet med ett fel och kan köras om. En worker som inte kan startas tömmer inte arbetskön; åtgärda felet och välj **Starta om motorn** under Arbetskö. En paus träder i kraft efter den pågående inspelningen.

Installationsskriptet använder `worker/requirements-linux-py312.lock`, som låser de verifierade Pythonberoendena för Linux och Python 3.12. `worker/requirements.txt` beskriver beroendena för framtida uppdateringar. PyAV är låst till 17.1.0 eftersom faster-whisper 1.2.1 använder `metadata_errors`, som [PyAV 19 tog bort](https://github.com/PyAV-Org/PyAV/releases/tag/v19.0.0). Resultatens metadata innehåller den faktiska faster-whisper- och CTranslate2-versionen, modellrevisionen och batchstorleken.

För engelska inspelningar hämtar du en separat modell en gång:

```bash
.venv/bin/python worker/prepare_model.py --model-id Systran/faster-whisper-large-v3 --output "$PWD/models/whisper-large-v3"
```

Ange mappen under **Inställningar → Engelsk modellmapp**. Välj **English** på inspelningen och välj **Ny transkribering** för att köra om en tidigare version. Tidigare transkript bevaras. Språket för ett pågående jobb kan inte ändras. Saknas den engelska modellen visas ett fel för det jobbet; svenska inspelningar kan fortfarande behandlas.

## Journal: från upplevelse till bild

1. Välj **Ny upplevelse** i Journal och ange titel, datum, dröm/meditation, språk och beskrivning. Datum är upplevelsens datum och kan ändras; ett inspelningsdatum gissas inte från filens ändringstid.
2. Alternativt öppnar du en inspelning, fäller ut textmarkeringen och väljer **Journalpost av markering** eller **Journalpost av hela transkriptet**. Texten kopieras och källversionen länkas. Granska och rätta kopian och välj **Bekräfta den granskade beskrivningen** innan du skapar utkast.
3. Skriv **Mina reflektioner** separat. De sparas med posten men används bara om du väljer **Ta med mina reflektioner i nästa utkast**. Detta är ett val för just nästa begäran.
4. Välj **Skapa scenutkast**. Efter lokal bearbetning visas separata scener med sammanfattning, källcitat, kreativa tillägg och engelska bildpromptar. Omöjlig geometri och drömlika motsägelser ska bevaras. Citat är granskningshjälp, inte en garanti att modellen förstått korrekt.
5. Redigera detaljer, tillägg, sammanfattningar eller prompten. Ta bort oönskade tillägg; om prompten är manuellt omskriven måste du även granska att tillägget försvunnit där. **Spara ny version** bevarar tidigare utkast. Varje ändrad version behöver godkännas igen.
6. Välj ett av dina importerade ComfyUI-flöden och **Godkänn granskad prompt**. Generera en scen eller **Generera alla scener separat** efter att samtliga scenpromptar granskats. Flödets modell, seed, bildstorlek och övriga inställningar bevaras. En orörd negativ prompt använder flödets standard; ett uttryckligen tömt fält skickar en tom prompt om flödet stöder det.
7. Markera minst två scener och välj **Kombinera valda scener**. Granska den nya kompositionen och förklaringen av hur scenerna passar ihop innan du godkänner och genererar. De individuella utkasten finns kvar.
8. Bilder och försök sparas på posten med sina exakta promptar och workflow-kopior. Markera en favorit. Borttagning raderar bara den lokala bilden och avmarkerar den om den var favorit.

Postens text autosparas. Utkastredigeringar sparas med **Spara ny version** och före navigering eller andra åtgärder. Osparade ändringar behålls i formuläret om lagringen misslyckas. Tidigare beskrivningar och utkast kan läsas i historiken. Utkast från en äldre beskrivning kan aldrig godkännas för den aktuella posten.

Om språkmodellen inte kan starta eller ett utkast misslyckas bevaras beskrivning och tidigare utkast. Välj **Försök igen** eller **Skriv en egen scenprompt**. För långa beskrivningar avvisas med en uppmaning att välja ett kortare avsnitt; de kapas inte tyst. Ett pågående utkast markeras som avbrutet efter en omstart och kan köras om manuellt.

## Installera lokal scenutkastning

Kräver den befintliga Pythonmiljön, Git, CMake, en C/C++-kompilator och CUDA-toolkit med `nvcc` (utöver NVIDIA-drivrutinen). Kör explicit en gång:

```bash
bash scripts/setup-drafting.sh
```

Skriptet bygger en CUDA-aktiverad llama-server i `.local/llama.cpp/` och hämtar den officiella **Qwen3-8B Q4_K_M**-modellen, cirka 5 GB, till `models/qwen3-8b/`. Första installationen låser de upplösta källrevisionerna; upprepade körningar behåller dem. Runtime-revision, version och binärens SHA-256 sparas i `.local/llama-runtime.json`. Modellrevision och SHA-256 sparas i `dreamwhisper-text-model.json`. Byt mapp vid modellbyte. Workern kör enbart lokala filer och gör inga automatiska nedladdningar.

Öppna **Inställningar → Lokal språkmodell för scenutkast** och kontrollera program- och modellsökvägarna. Utvecklingsläget förifyller projektets sökvägar; i en installerad app kan du behöva ange dem själv. Att filerna finns innebär inte att GPU-starten verifierats.

llama-server startas bara av supervisorn, på en lokal adress med ett tillfälligt API-lösenord, 16 384 tokens kontext, en begäran åt gången och tänkande avstängt. Källdetaljer extraheras först; kreativa tillägg föreslås därefter separat för varje scen. Kombinationer bevarar redan granskade detaljer och tillägg och föreslår en konkret placering i samma bild. Processen avslutas och inväntas efter varje utkast. Talmodell, språkmodell och ComfyUI använder GPU:n i tur och ordning. Redan inskickade bildjobb följs upp och ComfyUI avlastas innan andra motorer startar. En upptagen extern ComfyUI-kö avbryts inte; väntan/avlastningsfel visas för återförsök. Samordningen förutsätter fortfarande att andra program inte samtidigt skickar nya GPU-jobb.

Diagnostik finns i `logs/drafting.log`; journalens innehåll sparas i SQLite, inte avsiktligt i diagnostikloggen. Ingen molntjänst används. Symboliska metaforer, AI-tolkningar av betydelse och molnleverantörer ingår inte i denna första version.

Säkerhetskopiera appdatamappen när appen är avslutad före uppgradering. Databasversion 4 bevarar befintliga inspelningar, transkript, flöden och bildhistorik. Äldre appversioner kan inte öppna den uppgraderade databasen. Befintliga bilder blir inte automatiskt journalposter.

## Importera från Sony

1. Anslut diktafonen via USB.
2. Öppna **Diktafon** och välj **Registrera diktafon**.
3. Appen importerar WAV/MP3 från `REC_FILE` i volymens rot eller `PRIVATE/SONY/REC_FILE` och alla undermappar. Om båda inspelningsmapparna finns läses båda. `MUSIC` importeras inte. Även volymer med minneskort kan visas.
4. Vid återanslutning importeras ej arkiverat innehåll; datum används inte för att avgöra vad som är nytt.

Upptäckten använder UDisks2 `GetManagedObjects`, USB-/modellinformation, serienummer, volym-ID och Sony-mappstrukturen. Den lyssnar på UDisks2-signaler och inventerar dessutom var femte sekund för att återhämta missade händelser. En redan ansluten diktafon hittas vid appstart. Registrering med serienummer omfattar volymer på samma enhet. Saknas serienummer används volym-ID. Den fysiska ICD-UX570-enhetens identifierare behöver verifieras på din dator.

En omonterad diktafon monteras via UDisks2 med `ro`. Befintliga monteringar återanvänds. Om systemets behörigheter stoppar monteringen kan du montera diktafonen i filhanteraren. Appen ändrar eller raderar aldrig inspelningar på diktafonen.

I **Inspelningar** kan du dra och släppa flera WAV- eller MP3-filer från filhanteraren. Filerna kopieras, verifieras och läggs i transkriberingskön; originalen bevaras och dubbletter hoppas över. Mappar, länkar och andra filtyper rapporteras som fel utan att hindra övriga filer. Om transkriberingen är pausad väljer du **Starta kön** under Arbetskö.

**Importera ljud** i biblioteket tar en fullständig mappsökväg och fungerar även utan diktafon. Endast vanliga WAV/MP3-filer importeras; symboliska länkar följs inte. För närvarande finns ingen grafisk mappväljare.

## Arkiv, återhämtning och redigering

Tauri använder sin appdatamapp, vanligtvis `~/.local/share/se.dreamwhisper.desktop/`. Den exakta sökvägen visas i appen.

```text
appdatamapp/
  archive/                  # SHA-256-namngivna original och JSON-manifest
  dreamwhisper.sqlite3      # historik, kö, transkript och rättningar
  dreamwhisper.sqlite3-wal  # kan finnas medan appen körs
  dreamwhisper.sqlite3-shm
  exports/                  # TXT, Markdown, SRT
  images/                   # lokalt bildarkiv, ordnat per bildjobb
  logs/worker.log           # Pythonfel och diagnostik
  app.lock                  # en instans per arkiv
```

Importen kopierar till en temporär fil, läser och SHA-256-verifierar både kopia och källa, kontrollerar källans storlek/ändringstid, synkar till disk och publicerar filen genom atomiskt namnbyte. Ett manifest sparas före publicering. Inspelning och köjobb registreras tillsammans i en SQLite-transaktion. Samma ljudinnehåll ger en arkivpost, även om filnamnet ändrats. För säker verifiering läses även tidigare importerade källfiler vid en ny importsökning; en stor diktafon kan därför ta tid att inventera.

Vid start återställs pågående jobb till kön. Manifest återför färdigkopierade filer som inte hann registreras i databasen. Saknade arkivfiler blockeras från transkribering. Ofullständiga `.part`-filer efter ett hårt avbrott räknas aldrig som färdiga inspelningar; de kan ligga kvar tills de tas bort manuellt.

Biblioteket söker i filnamn och den senaste transkriptversionens text. Klicka på en tidsstämpel för att lyssna. Spara rättningar innan du lämnar inspelningen. Varje ny transkribering får en ny version; tidigare maskintext och rättningar finns kvar och kan väljas i versionsmenyn. Export använder den valda versionen och dess rättningar. Ordtidsstämplar avser maskintexten och justeras inte automatiskt när du redigerar. Visat datum är källfilens ändringstid och ska inte ses som ett säkert inspelningsdatum.

Ljudspelaren läser arkivfilen via Tauri:s begränsade asset-protokoll och spelar en lokal blob-URL, för att kringgå problem med direkt `asset://`-uppspelning i WebKitGTK på Linux. CSP tillåter denna lokala läsning och blob-uppspelning. Den komprimerade ljudfilen hålls i minnet medan den är vald; blob-URL:en frigörs när inspelning eller vy byts. Datorn behöver GStreamer med MP3/WAV-avkodning. Ljudinläsningen visar separata fel för saknad fil, nekad åtkomst och avkodningsproblem.

Säkerhetskopiera hela appdatamappen när appen är avslutad. Modellen och Pythonmiljön kan återskapas separat.

## Skapa bilder med ComfyUI

1. Under **Inställningar → ComfyUI – server**, ange serveradress, ComfyUI-mapp och dess Pythonprogram. Klicka **Starta ComfyUI-server**. På denna dator hittas `~/comfy/ComfyUI` och dess `.venv/bin/python` automatiskt. Du kan också starta ComfyUI separat.
2. Exportera ditt text-till-bild-workflow från ComfyUI i **API-format** (Export API / Save API Format). Vanligt visuellt workflow-JSON stöds inte.
3. Under **Journal → Scener och bildpromptar** eller **Bilder → Mina flöden**, välj **Lägg till flöde**, importera JSON-filen, ge flödet ett namn och välj nodens textfält för positiv bildprompt och, om flödet har det, textfält för negativ prompt. Klicka **Spara flöde**. Du kan spara flera flöden och välja ett i menyn **Flöde för nästa bild**. Övriga workflow-värden, inklusive seed, negativa promptar, modellnamn och bildstorlek, behålls. **Ändra namn / promptfält** kan också ersätta JSON-filen för ett befintligt flöde.
4. Öppna en inspelning och fäll ut **Markera text för en bild**. Markera text över ett eller flera stycken och välj **Skapa bild av markering**.
5. I **Bilder** kan du skriva om eller komplettera prompten innan du väljer **Skapa bild**. Du kan också börja med en helt egen prompt. Under Bilder finns separata fält för positiv och negativ prompt. För ett tidigare sparat flöde väljer du negativt promptfält via **Ändra namn / promptfält**. Den negativa prompten förifylls från flödet tills du ändrar den; ett uttryckligen tömt fält skickar en tom negativ prompt. Båda promptarna sparas i utkast och bildhistorik. Bildprompten ändrar aldrig transkriptet.

Promptutkast sparas lokalt. Varje bildjobb sparar källtext, redigerad prompt, eventuell inspelning/transkriptversion och en egen workflow-kopia. Bildgalleriet visar alla PNG/JPEG/WebP-resultat som workflow rapporterar; använd SaveImage eller PreviewImage. Klicka på en bild för större visning. Bilderna kopieras till appens `images/`-mapp och kan visas efter omstart även om ComfyUI är avstängt. Databasen migreras automatiskt till version 4 och ett tidigare konfigurerat workflow flyttas till Mina flöden; äldre appversioner kan inte öppna den uppgraderade databasen.

Whisper och bildjobben delar en supervisor: pågående transkribering slutförs, Whisper-processen avslutas och därefter skickas bildjobbet. När bildjobbet är avslutat väntar appen tills ComfyUI-kön är tom, begär modellavlastning via `/free` och låter Whisper återstarta om transkribering fortfarande är aktiverad. Avlastningen är asynkron; om Whisper inte kan återstarta visas felet i Arbetskö. En manuell paus respekteras. Samordningen förutsätter att andra program inte samtidigt skickar nya GPU-jobb till ComfyUI.

Vid omstart eller nätverksfel följs redan skickade jobb upp via server-ID utan automatisk återsändning. **Följ upp / hämta igen** hämtar tidigare resultat utan ny bildkörning. Om servern inte längre går att nå kan **Avsluta uppföljning** användas efter att du kontrollerat att ComfyUI inte kör jobbet; detta avbryter inte jobbet i ComfyUI. Appen behöver en aktuell lokal ComfyUI som accepterar `prompt_id` i `/prompt` (den lokalt installerade serverkoden gör det). ComfyUI installeras inte av DreamWhisper. Startknappen kör installationens Pythonprogram och `main.py` på en lokal adress med webbläsarstart avstängd. Status visar när servern svarar; startfel finns i appdatamappens `logs/comfyui.log`. En redan körande server återanvänds. DreamWhisper avslutar endast sin egen server när appen avslutas via systemfältet; en separat startad server lämnas igång. Att bara stänga appfönstret avslutar inte servern. Journalens lokala språkmodell skapar engelska bildpromptar efter separat installation enligt nedan. Fristående bildpromptar använder fortfarande dina egna texter.

## Testa och bygga

```bash
npm run build
npm test
cargo check --manifest-path src-tauri/Cargo.toml --offline
cargo clippy --manifest-path src-tauri/Cargo.toml --no-default-features --offline -- -D warnings
npm run tauri -- build
```

Tester täcker hashverifiering, dubbletter, manifeståterhämtning, köåterställning, saknade arkivfiler, versionsbevarande, svenska sökningar, SRT-export, instanslås och workerprotokoll med batch-fallback. Bildtesterna täcker workflow-fältersättning, markerad text, beständiga utkast, API-fel, återhämtning utan dubbla jobb, flera bildresultat och GPU-överlämning med en simulerad Pythonprocess och lokal HTTP-testserver. Testerna behöver tillåtelse att öppna en lokal testport. Pythonprotokolltesterna använder en simulerad talmodell; de verifierar inte verklig GPU-prestanda eller transkriptionskvalitet.

För en diagnostisk inventering via verklig UDisks2:

```bash
cargo run --manifest-path src-tauri/Cargo.toml --no-default-features --example devices
```

För ett verkligt GPU-test med en egen inspelning i ett isolerat tillfälligt arkiv:

```bash
cargo run --manifest-path src-tauri/Cargo.toml --no-default-features --example transcribe -- "$PWD/.venv/bin/python" "$PWD/models/kb-whisper-large" /full/sökväg/inspelning.wav
```

För att kontrollera GPU-kärnorna med syntetiskt ljud:

```bash
.venv/bin/python scripts/check-gpu.py
```

## Kodstruktur

- `src/`: TypeScriptgränssnitt, uppspelning och redigering.
- `src-tauri/src/archive.rs`: säker kopiering och återhämtning.
- `src-tauri/src/db.rs`: SQLite, beständig kö och transkriptversioner.
- `src-tauri/src/images.rs`: beständiga bildjobb, ComfyUI-API och lokalt bildarkiv.
- `src/images.ts`: prompteditor, workflow-inställningar och bildgalleri.
- `src/journal.ts` och `src-tauri/src/journal.rs`: journalposter, revisionshistorik, granskning och godkända bildjobb.
- `src-tauri/src/drafting.rs`: lokal scenextraktion, separata kreativa tillägg och processlivscykel.
- `src-tauri/src/devices.rs`: UDisks2 och enhetsidentifiering.
- `src-tauri/src/service.rs`: bakgrundsimport och seriell arbetskö.
- `src-tauri/src/worker.rs`: Pythonprocessens livscykel och JSON-protokoll.
- `src-tauri/src/autostart.rs`: valfri XDG-start vid inloggning.
- `src-tauri/src/desktop.rs`: Tauri-kommandon, lokalt ljudprotokoll och systemfält.
- `worker/`: modellhämtning och offline-transkribering.

Verifierat på denna dator: desktopbygge, TypeScriptbygge, Clippy och 70 automatiska tester (47 Rust, 6 Python och 17 TypeScript/JavaScript). Native-gränssnittet har startats och granskats i ett isolerat testarkiv. Den lokala Qwen3-8B-modellen och CUDA-runtime är installerade och verifierade på RTX 3090 (24 GB). Svenska och engelska syntetiska beskrivningar gav två separata scener på cirka 7–9 sekunder, med engelska promptar och granskningsmaterial på respektive språk. Ett verkligt GPU-test kombinerade scenerna i en föreslagen vänster/höger-komposition och behöll källdetaljerna. Dessa tester verifierar körningen, inte bildkvaliteten.

Tidigare verifierades import → SQLite-kö → lokal KB-Whisper large CUDA FP16 → resultatlagring med en tyst 3-sekunders WAV. Tyst ljud gav noll textsegment med VAD. Ett separat test med ett explicit ljudfönster verifierade GPU-inferens och ordtidsstämpling; det mäter inte taligenkänningens kvalitet. Den engelska large-v3-modellen är också installerad, men dess taligenkänningskvalitet återstår att bedöma.

Återstår för användarvalidering: de fem egna upplevelsernas bilder, riktiga svenska/engelska talinspelningar, ordtidsstämplarnas kvalitet och fysisk Sony-anslutning med dess volymidentifierare. Symboliskt läge kommer efter att det första journalflödet klarat acceptanstestet.

Arbetskö visar pågående och väntande inspelningar i behandlingsordning samt misslyckade jobb med felorsak. Välj **Starta kön** för att aktivera transkribering; **Pausa kön** stoppar efter pågående inspelning. Försök igen köar om filen och bevarar tidigare transkript, men aktiverar inte en pausad kö. Vid fel när modellen laddas kan du korrigera inställningarna och välja **Starta om motorn**.

Under **Inställningar → Sparade bildflöden** kan du ta bort flöden. Bildhistorik, sparade bilder och redan köade jobb behåller sina workflow-kopior. Om det valda flödet tas bort väljs nästa sparade flöde (eller det föregående om det var sist).

Bilder visar de fem senaste enskilda bilderna, och varje bildjobb visar sina egna bilder. Under **Galleri** finns alla sparade bilder med de senaste först. **Ta bort bild** raderar den lokala kopian efter bekräftelse; ComfyUI-originalet och övriga bilder i samma jobb behålls. Borttagna bilder hämtas inte tillbaka vid uppföljning av jobbet.

## Validera journalflödet

Automatiska tester använder även en lokal CPU-testserver för att kontrollera strukturerade språkmodellsvar, språkval, för stora indata, startfel, avstängning och processavslut. De verifierar inte bildlikhet eller taligenkänningens kvalitet.

Efter installation kan du prova verklig GPU-utkastning med en egen textfil i ett isolerat temporärt arkiv:

```bash
cargo run --manifest-path src-tauri/Cargo.toml --no-default-features --example draft -- "$PWD/.local/llama.cpp/build/bin/llama-server" "$PWD/models/qwen3-8b/Qwen3-8B-Q4_K_M.gguf" sv /full/sökväg/beskrivning.txt
```

Använd `en` för engelska och lägg till `--combine` för att även prova en kombinerad komposition. Diagnostikexemplet skriver ut de genererade scenutkasten och ändrar inte ditt vanliga arkiv.

Produktens acceptanstest är fem riktiga drömmar eller meditationer, med både språk, ljud/text och flera scener representerade. Anteckna viktiga detaljer före generation. Minst fyra ska ge en bild du vill spara, med detaljerna bevarade och högst en större promptomskrivning per upplevelse. Bedöm symboliskt läge separat när det senare implementeras.
