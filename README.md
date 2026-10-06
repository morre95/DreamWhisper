# DreamWhisper

En Linuxapp för eget bruk som importerar Sony ICD-UX570-inspelningar, bevarar originalen och transkriberar svenska lokalt med KB-Whisper large.

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

Workern kör offline och laddar inte ner modeller. Den håller modellen laddad och kör ett jobb åt gången med svenska, CUDA, FP16, VAD och ordtidsstämplar. Vid GPU-minnesbrist halveras batchstorleken ner till 1. Om en fil fortfarande misslyckas behålls den i arkivet med ett fel och kan köras om. En worker som inte kan startas tömmer inte arbetskön; åtgärda felet och stäng av/slå på transkribering. En paus träder i kraft efter den pågående inspelningen.

Installationsskriptet använder `worker/requirements-linux-py312.lock`, som låser de verifierade Pythonberoendena för Linux och Python 3.12. `worker/requirements.txt` beskriver beroendena för framtida uppdateringar. PyAV är låst till 17.1.0 eftersom faster-whisper 1.2.1 använder `metadata_errors`, som [PyAV 19 tog bort](https://github.com/PyAV-Org/PyAV/releases/tag/v19.0.0). Resultatens metadata innehåller den faktiska faster-whisper- och CTranslate2-versionen, modellrevisionen och batchstorleken.

## Importera från Sony

1. Anslut diktafonen via USB.
2. Öppna **Diktafon** och välj **Registrera diktafon**.
3. Appen importerar WAV/MP3 från `REC_FILE` i volymens rot eller `PRIVATE/SONY/REC_FILE` och alla undermappar. Om båda inspelningsmapparna finns läses båda. `MUSIC` importeras inte. Även volymer med minneskort kan visas.
4. Vid återanslutning importeras ej arkiverat innehåll; datum används inte för att avgöra vad som är nytt.

Upptäckten använder UDisks2 `GetManagedObjects`, USB-/modellinformation, serienummer, volym-ID och Sony-mappstrukturen. Den lyssnar på UDisks2-signaler och inventerar dessutom var femte sekund för att återhämta missade händelser. En redan ansluten diktafon hittas vid appstart. Registrering med serienummer omfattar volymer på samma enhet. Saknas serienummer används volym-ID. Den fysiska ICD-UX570-enhetens identifierare behöver verifieras på din dator.

En omonterad diktafon monteras via UDisks2 med `ro`. Befintliga monteringar återanvänds. Om systemets behörigheter stoppar monteringen kan du montera diktafonen i filhanteraren. Appen ändrar eller raderar aldrig inspelningar på diktafonen.

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
  logs/worker.log           # Pythonfel och diagnostik
  app.lock                  # en instans per arkiv
```

Importen kopierar till en temporär fil, läser och SHA-256-verifierar både kopia och källa, kontrollerar källans storlek/ändringstid, synkar till disk och publicerar filen genom atomiskt namnbyte. Ett manifest sparas före publicering. Inspelning och köjobb registreras tillsammans i en SQLite-transaktion. Samma ljudinnehåll ger en arkivpost, även om filnamnet ändrats. För säker verifiering läses även tidigare importerade källfiler vid en ny importsökning; en stor diktafon kan därför ta tid att inventera.

Vid start återställs pågående jobb till kön. Manifest återför färdigkopierade filer som inte hann registreras i databasen. Saknade arkivfiler blockeras från transkribering. Ofullständiga `.part`-filer efter ett hårt avbrott räknas aldrig som färdiga inspelningar; de kan ligga kvar tills de tas bort manuellt.

Biblioteket söker i filnamn och den senaste transkriptversionens text. Klicka på en tidsstämpel för att lyssna. Spara rättningar innan du lämnar inspelningen. Varje ny transkribering får en ny version; tidigare maskintext och rättningar finns kvar och kan väljas i versionsmenyn. Export använder den valda versionen och dess rättningar. Ordtidsstämplar avser maskintexten och justeras inte automatiskt när du redigerar. Visat datum är källfilens ändringstid och ska inte ses som ett säkert inspelningsdatum.

Ljudspelaren läser arkivfilen via Tauri:s begränsade asset-protokoll och spelar en lokal blob-URL, för att kringgå problem med direkt `asset://`-uppspelning i WebKitGTK på Linux. CSP tillåter denna lokala läsning och blob-uppspelning. Den komprimerade ljudfilen hålls i minnet medan den är vald; blob-URL:en frigörs när inspelning eller vy byts. Datorn behöver GStreamer med MP3/WAV-avkodning. Ljudinläsningen visar separata fel för saknad fil, nekad åtkomst och avkodningsproblem.

Säkerhetskopiera hela appdatamappen när appen är avslutad. Modellen och Pythonmiljön kan återskapas separat.

## Testa och bygga

```bash
npm run build
npm test
cargo check --manifest-path src-tauri/Cargo.toml --offline
cargo clippy --manifest-path src-tauri/Cargo.toml --no-default-features --offline -- -D warnings
npm run tauri -- build
```

Tester täcker hashverifiering, dubbletter, manifeståterhämtning, köåterställning, saknade arkivfiler, versionsbevarande, svenska sökningar, SRT-export, instanslås och workerprotokoll med batch-fallback. Pythonprotokolltesterna använder en simulerad talmodell; de verifierar inte verklig GPU-prestanda eller transkriptionskvalitet.

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
- `src-tauri/src/devices.rs`: UDisks2 och enhetsidentifiering.
- `src-tauri/src/service.rs`: bakgrundsimport och seriell arbetskö.
- `src-tauri/src/worker.rs`: Pythonprocessens livscykel och JSON-protokoll.
- `src-tauri/src/autostart.rs`: valfri XDG-start vid inloggning.
- `src-tauri/src/desktop.rs`: Tauri-kommandon, lokalt ljudprotokoll och systemfält.
- `worker/`: modellhämtning och offline-transkribering.

Verifierat på denna dator: desktopbygge, TypeScriptbygge, Clippy, 13 automatiska tester, UDisks2-anrop och import → SQLite-kö → lokal KB-Whisper large CUDA FP16 → resultatlagring med en tyst 3-sekunders WAV på RTX 3090 (24 GB). Tyst ljud gav korrekt noll textsegment med VAD. Ett separat test med ett explicit ljudfönster verifierade också GPU-inferens och ordtidsstämpling i FP16; det testet mäter inte taligenkänningens kvalitet. Modellrevision: `d5d5984b4d8f7c4847a8ea203f1976285fb28300`.

Återstår för validering: fysisk Sony-anslutning, dess volymidentifierare, svenska talinspelningar, ordtidsstämplarnas kvalitet och visuell kontroll av native-gränssnittet. Ingen diktafon eller automatiserbar GUI-session fanns tillgänglig under implementationen. Nästa förbättringar är grafisk mappväljare, valbar arkivplats och exaktare inspelningsdatum från Sony-filnamn.

Arbetskö visar pågående och väntande inspelningar i behandlingsordning samt misslyckade jobb med felorsak. Välj **Starta kön** för att aktivera transkribering; **Pausa kön** stoppar efter pågående inspelning. Försök igen köar om filen och bevarar tidigare transkript, men aktiverar inte en pausad kö. Vid fel när modellen laddas kan du korrigera inställningarna och välja **Starta om motorn**.
