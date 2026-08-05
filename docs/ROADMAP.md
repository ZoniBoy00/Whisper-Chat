# Whisper (e2ee-chat) — Tekninen Tiekartta

> Whisper — "kuiskaus". Viestit ovat kuiskauksia: vain lähettäjä ja vastaanottaja
> kuulevat. Privacy-first, end-to-end-encrypted (E2EE) yleiskäyttöinen chat —
> WhatsApp/Signal/Telegram -korvaaja, ilman takaovia ja skannausmekanismeja.
>
> **Päiväys:** 2026-08-05 (päivitetty: brändi, tuotevisio, hardening, deploy-suojaus)
> **Tila:** Suunnittelu (Phase 1) — relay + kryptoydin koodattu ja testattu
> **Alkuperä:** Sovellusmäärittely + Gemini-ristiintarkistus + Byte-arviointi
> **Työnimi (ennen brändäystä):** Operation Ghost

---

## 1. Tavoite ja tuotevisio

**Yleiskäyttöinen, helppo, moderni chat** joka korvaa WhatsAppin, Signalin ja Telegramin — ei tekninen lelu. Tinkimätön E2EE kaikessa: viestit, ryhmät, media, puhelut.

| Ominaisuus | Tavoite |
|---|---|
| **Helppo käyttöönotto** | Asenna → avainpari luodaan automaattisesti → jaa oma ID kutsulinkkinä. Ei puhelinnumeroa, ei sähköpostia, ei tilejä. |
| **Moderni UI** | Puhdas, minimalistinen, Signal/Telegram-taso. Dark theme, Lucide-ikonit, nopea ja responsiivinen. |
| **Kaikki E2EE** | 1:1 viestit, ryhmät, media, tiedostot, ääni- ja videopuhelut. Palvelin ei näe mitään sisällöstä. |
| **Turvallinen oletuksena** | Secure by default — ei opt-in -turvaa. Vahvistetut kontaktit (safety numbers), katoavat viestit. |
| **Kevyt & nopea** | Desktop < 20 MB, akkuystävällinen mobiili, nopea synkronointi. |
| **Kestävä sääntelyä vastaan** | Arkkitehtuuri tekee sisällön skannauksesta teknisesti mahdotonta (ei "yhtä valitsinta" jota säännellä). |

**Metadata-malli (päätös):** Ei Tor-relayta, ei onion-reititystä, ei mixnetiä. Palvelin näkee reititystiedot (kuka puhuu kelle + IP) — **täsmälleen kuten Signal ja WhatsApp**. Yksityisyys tulee E2EE:stä, ei verkon piilottamisesta. Tämä on hyväksytty ja rehellinen malli normaalikäyttäjälle.

## 2. Keskeiset periaatteet

| Periaate | Mitä se tarkoittaa käytännössä |
|---|---|
| **Zero-Knowledge (rehellisesti)** | E2EE suojaa **sisällön**. Palvelin näkee vain salatut enveloppet, peer ID:t ja liikennemäärät. Metatiedot palvelimella ovat olemassa (välitys vaatii) — dokumentoitu rehellisesti. |
| **Zero-Trust** | Mikään yksittäinen komponentti ei voi vuotaa keskusteluja. Jokainen kerros oletetaan vihamieliseksi. |
| **Ei omaa kryptoa** | Kaikki kryptografia tulee todistetusta, auditoidusta kirjastosta (`vodozemac`). Oma krypto = varma katastrofi. |
| **Helppous ei saa rikkoa turvaa** | Turvallinen oletuksena; turva ei vaadi käyttäjältä teknistä osaamista. |
| **Kevyt & nopea** | Desktop-binaari < 20 MB, muistinkulutus murto-osa Electronista, akkuystävällinen mobiili. |
| **Ei takaovia** | Ei palvelinpuolen skannausta, ei E2EE-kiertoteitä, ei telemetriaa. |
| **Scope-kuri** | MVP = Desktop + relay + 1:1 E2EE. Ryhmät, media, puhelut ja mobiili eivät kuulu MVP:hen. |

**Miksi tämä kestää Chat Control -tyyppisen sääntelyn?**
Chat Control / client-side scanning vaatii skannausmekanismin asiakassovellukseen. Koska Whisperin asiakassovellus ei **pysty** näkemään sisältöä (avaimet Double Ratchet -istunnossa, viestit salataan ennen levylle kirjoittamista), ei ole olemassa "yhtä valitsinta" jota säännellä. Sisällön skannaus ei ole mahdollista ilman koko sovelluksen uudelleenkirjoitusta.

---

## 3. Lukitut päätökset

| # | Päätös |
|---|---|
| 1 | 🛑 **Ei Double Ratchet -toteutusta itse** → **`vodozemac`** (Apache-2.0, auditoitu). `libsignal` (AGPL) hylätty. |
| 2 | **Ei onion/Tor** → Signal/WhatsApp-malli: E2EE sisällölle, palvelin näkee reitityksen. |
| 3 | **Scope-kuri** → MVP = Desktop + relay + 1:1 E2EE. |
| 4 | **"Zero-knowledge" määritelty rehellisesti** → sisältö suojattu, metatiedot olemassa. |
| 5 | **Nimi:** **Whisper** (työnimi Operation Ghost). Repo `ZoniBoy00/e2ee-chat`, lisenssi MIT. |

---

## 4. Teknologiapino (Tech Stack)

**Ydinperiaate: yksi jaettu Rust-ydin (`e2ee-core`), jota käyttävät** Tauri desktop (natiivina), Flutter mobiili (`flutter_rust_bridge`) ja testit. Salauslogiikka kirjoitetaan **kerran**, testataan **kerran**, jaetaan kaikkialle.

| Kerros | Valinta | Perustelu |
|---|---|---|
| **Jaettu ydin** | Rust crate **`e2ee-core`** | Sama salaus/protokolla kaikille alustoille |
| **Salaus** | **vodozemac** (X3DH + Double Ratchet) | Apache-2.0, auditoitu, ei omaa kryptoa |
| **Relay/backend** | Rust + **axum + tokio + WebSocket** | Kevyt, nopea, turvallinen muistinhallinta |
| **Palvelin-DB** | **SQLite** (vain ciphertext-envelopet + prekeyt, TTL) | Ei salaamatonta dataa koskaan levylle |
| **Desktop** | **Tauri v2** (Rust-ydin + React/TS) | ~10 MB vs Electron 100+ MB |
| **UI** | **React + TypeScript + Tailwind + shadcn/ui + Lucide-ikonit** | Moderni, siisti, nopea kehitys, dark theme |
| **Lokaali DB (client)** | rusqlite + **SQLCipher** | Salattu paikallinen historia |
| **Mobiili (vaihe 8)** | **Flutter + flutter_rust_bridge** | Yksi koodikanta iOS+Android, sama Rust-ydin |
| **Puhelut (vaihe 7)** | WebRTC (DTLS-SRTP) + coturn (STUN/TURN) | P2P, salattu media |
| **Transport** | TLS 1.3 + sertifikaattipinning | Suojaa liikenteen päästä päähän palvelimelle |
| **Deploy** | Hetzner VPS + systemd (kovetettu), GitHub Actions CI | Olemassa oleva infra |
| **Testaus** | `cargo test` (TDD), `clippy -D warnings`, `proptest`, `cargo audit` | Krypto ilman testejä = ei mergeä |

### Miksi ei X?

| Vaihtoehto | Miksi ei |
|---|---|
| Electron | 100+ MB binaari, 200+ MB RAM. Rikkoo "kevyt ja nopea" -arvon. |
| React Native | JS-silta hidastaa kryptoa, kaksi riippuvuuskaaosta. |
| Go backend | Valid, mutta Rust antaa suorituskyvyn **ja** muistiturvallisuuden + jaetun kryptoytimen. |
| libsignal (AGPL) | AGPL pakottaa koko projektin samaan lisenssiin. |

---

## 5. Kryptografinen suunnitelma

### Identiteetti (ei henkilötietoja)
- **Avainpari:** X25519 (avaintenvaihto) + Ed25519 (allekirjoitukset).
- **Peer ID:** hash identiteettijulkisavaimesta (SHA-256 → 16 hex) — ei puhelinnumeroa, ei nimeä.
- **Safety Numbers:** kontaktien vahvistus QR-koodilla/numerosarjalla (kuten Signal). Ei luottamusta palvelimeen.

### Istunnon muodostus (X3DH) ja viestit (Double Ratchet)
1. Alice hakee Bobin **prekey-bundlen** (allekirjoitettu, tamper-suojattu — ks. `e2ee-core/src/prekey.rs`).
2. X3DH-kättely → root key → **Double Ratchet -istunto**.
3. Viestit salataan **AES-256-GCM**/**ChaCha20-Poly1305**, avaimet **HKDF-SHA256**. Täysi forward & backward secrecy.
4. **Kirjasto:** `vodozemac` (LUKITTU). **TDD-vaatimus:** kryptomuutos ilman testejä = ei mergeä.

### Ryhmät (vaihe 6) & Post-Quantum (vaihe 9+)
- Ryhmät: **Sender Keys / Megolm**. Tulevaisuus: MLS (RFC 9420).
- PQ: **X25519Kyber768** hybridi — "harvest now, decrypt later" -suoja. Ei v1:ssä, mutta tilaa jätetty.

---

## 6. Arkkitehtuuri

### 6.1 Kaavio

```
┌─────────────┐     WebSocket (TLS 1.3)     ┌──────────────────┐
│  Tauri App  │ ───────────────────────────▶ │  Whisper Relay   │
│  (Rust+TS)  │ ◀─────────────────────────── │  (axum + tokio)  │
└─────┬───────┘      ciphertext only         └────────┬─────────┘
      │                                              │
      │ e2ee-core (Rust)                             │ SQLite
      │  ├─ vodozemac (X3DH+Ratchet)                │ (envelopet,
      │  ├─ wire-protocol (serde, versionoitu)       │  prekeyt,
      │  └─ local store (SQLCipher)                  │  userit)
      └──────────────────────────────────────────────┘
```

### 6.2 Viestin elinkaari
1. Lähettäjä luo/päivittää Double Ratchet -sessionin vastaanottajan prekeyllä.
2. Viesti salataan → ciphertext-envelope.
3. Envelope relaylle → tallennus (SQLite, TTL 7 vrk) + välitys.
4. Vastaanottaja purkaa sessionin → **palvelin ei näe mitään**.

### 6.3 Palvelimen rooli & metadata (hyväksytty malli)

Palvelin näkee vain `{ sender, recipient, payload: <opaque ciphertext>, seq }`. Ei viestihistoriaa plaintextinä, ei profiileja, ei analyytiikkaa. Reititystiedot (kuka puhuu kelle, IP) näkyvät kuten Signalissa/WhatsAppissa — tämä on **hyväksytty tuotevalinta**, ei puute.

| Taso | Whisper (v1) |
|---|---|
| Viestin sisältö | 🔒 E2EE (Double Ratchet) |
| Reititys | ⚠️ Palvelin näkee peer ID:t (välitys vaatii) — kuten Signal |
| IP-osoitteet | ⚠️ Palvelin näkee IP:t (TLS suojaa matkan) |
| Liikennemallit | ⚠️ Näkyy → kevyt padding (valinnainen myöhemmin) |

---

## 7. Kehitysvaiheet (Phases)

> Jokainen vaihe = oma pipeline-run (Planner → Coder(s) → Tester → Reviewer → Reporter). Välissä integraatiotesti.

| Vaihe | Sisältö | Agentit | Status |
|---|---|---|---|
| **0** | Repo + workspace, CI, AGENTS.md | 1 | ✅ **tehty** |
| **1** | **Salausydin `e2ee-core`** (vodozemac: identiteetti, prekeyt, X3DH, ratchet, wire v1, signed hello) | 2 | ✅ **tehty — 23/23 testiä** |
| **2** | **Relay-palvelin** (WebSocket, SQLite offline-jono, `fetch_since`, rate limiting, signed hello + spoofing-suojaus) | 1 | ✅ **tehty** |
| **3** | **Desktop-kuori:** Tauri v2 + React/TS + shadcn/ui — kirjautumisnäkymä (avainpari lokaalisti), kontaktiview, chat-näkymä | 1 | 🟡 **kuori tehty** (identity-commandit, onboarding, layout) — ⚠️ Windows-linkitys vaatii MSVC Build Tools; CI (Linux) toimii |
| **4** | **E2EE-integraatio:** 1:1-viestit päästä päähän (prekey-vaihto → session → salaus → relay → purku) | 1–2 | ⏳ |
| **5** | **UI/UX:** moderni, minimalistinen, helppo — Signal/Telegram-taso (dark theme, Lucide, kutsulinkit, contactit, tila-indikaattorit) | 1–2 | ⏳ |
| **6** | **Ryhmät + katoavat viestit:** Megolm, disappearing messages | 2 | 🔒 MVP:n jälkeen |
| **7** | **Media + puhelut:** salattu tiedostonsiirto, WebRTC (DTLS-SRTP) + coturn | 2 | 🔒 MVP:n jälkeen |
| **8** | **Mobiili:** Flutter + flutter_rust_bridge; push (APNs/FCM — vain "sinulla on viesti") | erillinen | 🔒 MVP:n jälkeen |
| **9** | **Auditointi + PQ:** cargo audit, fuzz, ulkoinen review, threat model, X25519Kyber768 | — | 🔒 MVP:n jälkeen |

**MVP = vaiheet 0–5.** Realistinen aikataulu pipeline-työllä: **2–4 viikkoa**.

---

## 8. Code Hardening (anti-debug / anti-reverse)

Rust on jo lähtökohtaisesti huomattavasti vaikeampi reverseata kuin JS/Python. Tavoite: **nostaa kynnystä niin korkealle, ettei kukaan vaivaudu** — 100 %:a suojaa ei ole olemassa, eikä sellaista luvata.

| Tekniikka | Missä | Tila |
|---|---|---|
| **Release-profiilit:** `opt-level=3`, `lto="fat"`, `codegen-units=1`, `panic="abort"`, `strip=true` | workspace Cargo.toml | ✅ tehty |
| **Ei debug-symboleja tuotannossa** | release-buildit | ✅ (strip) |
| **Krypto pelkästään Rustissa** (ei JS-kerroksessa) | e2ee-core | ✅ arkkitehtuurissa |
| **UI-ilman salaisuuksia:** React-kerros ei koskaan käsittele avaimia — vain salatut blobit | desktop | ⏳ vahvistetaan vaiheessa 3 |
| **Mobiili-hardening:** Android R8/ProGuard + native strip, iOS strip + code signing | mobiili | 🔒 vaihe 8 |
| **Integrity/tamper-evidence** (binäärin eheystarkistus, harkinnanvarainen) | desktop | 🔒 vaihe 9 |
| **Cargo audit + riippuvuuskuri** (minimoidaan riippuvuuksia = pienempi hyökkäyspinta) | koko projekti | ⏳ jatkuva |

**Sääntö agentille:** release-buildit aina stripatuilla symboleilla, `panic="abort"` ei saa olla "unwind" tuotannossa, salaisuuksia ei koskaan JS-/UI-kerrokseen.

---

## 9. Palvelimen suojaus (deploy)

| Toimenpide | Kuvaus |
|---|---|
| **Oma käyttäjä ilman sudoa** | Palvelu pyörii `whisper`-käyttäjänä, ei rootina |
| **systemd-hardening** | `NoNewPrivileges`, `ProtectSystem=strict`, `PrivateTmp`, `ProtectHome`, tyhjä `CapabilityBoundingSet` — unit-template repoissa (`server/deploy/`) |
| **Vain tarvittavat portit auki** | ufw: 443 (ja 80 redirect) tai Cloudflare Tunnel; SSH vain avaimella + fail2ban |
| **TLS 1.3** | Certbot/Let's Encrypt, HSTS, sertifikaattipinning clientissä |
| **Rate limiting + koko-rajoitukset** | ✅ tehty (token bucket 60/min/IP, 8 MiB enveloppe-korkki) |
| **Ei viestisisältölokeja** | Logit vain peer ID -tasolla, log-rotatointi, `RUST_LOG=info` |
| **Salaisuudet** | `.env` + `WHISPER_DB_PATH`; `.env.example` dokumentoitu, ei koskaan repoihin |
| **Päivitykset** | Automaattinen turvapäivitys (unattended-upgrades), `cargo audit` CI:ssä |
| **Tietokanta** | Vain ciphertext-envelopet, TTL 7 vrk, 500/peer-korkki, `server/data/` gitignored |

---

## 10. Multi-Agent-työnjako

| Agentti | Missä | Rooli |
|---|---|---|
| **OpenCode + Agnes** | Jonin PC | Pääkoodari — feature-lanet |
| **Byte (minä)** | VPS/Hermes | Suunnittelu, arkkitehtuuri, testit, review, infra, deploy |
| **OpenCode (2. instanssi)** | Jonin PC | Rinnakkainen feature-lane / review |

**Per vaihe:** Joni → "mene" → bite-sized taskit → rinnakkaiset coderit (max 3) → testeri → reviewer → raportti → hyväksyntä → push.

---

## 11. Infra & Repo

- **Relay:** Hetzner VPS + systemd (`whisper-relay.service`), Cloudflare Tunnel tai suora portti.
- **CI:** GitHub Actions — `cargo test --workspace`, `clippy -D warnings`, `fmt` joka push.
- **Repo:** `ZoniBoy00/e2ee-chat` (yksityinen aluksi, julkinen myöhemmin). **Lisenssi: MIT.**
- **Nimeäminen:** Brändi **Whisper**; tekniset nimet `e2ee-core` (ydin) ja `whisper-relay` (relay). Rename tehty 2026-08-05.

---

## 12. Riskit & Realismi

| Riski | Lievennys |
|---|---|
| Krypto on vaikeaa | vodozemac + TDD + review + audit |
| Metadata näkyy palvelimelle | **Hyväksytty tuotevalinta** (Signal-malli) — rehellisesti dokumentoitu |
| Scope creep | Vaiheet 6–8 rajattu MVP:n ulkopuolelle |
| DoS / spämmi | Rate limiting, koko-korkki, signed hello (seuraava), TTL |
| Store-säännöt (skannauspaine) | Puhdas arkkitehtuuri; E2EE suojattu laissa (EU) |
| Anti-reverse ei ole absoluuttista | Kynnystä nostetaan, ei luvata mahdotonta |

**Avoimet kysymykset:**
1. Web (selain) -tuki? (WASM mahdollista — ei MVP:ssä)
2. Desktop-push-ilmoitukset MVP:hen?

---

## 13. Status & seuraavaksi (TODO)

**✅ Tehty (2026-08-05):**
- [x] Vaihe 0: workspace, CI, AGENTS.md, .gitignore
- [x] Vaihe 1: `e2ee-core` (vodozemac) — 23/23 testiä
- [x] Vaihe 2: relay + SQLite + fetch_since + rate limit + signed hello (spoofing-suojaus) — 19/19 testiä, smoke 15/15
- [x] Code hardening: release-profiilit, systemd-template, .env.example
- [x] Rename `ghost-relay` → `whisper-relay` (crate, env-muuttujat `WHISPER_*`, deploy-unit)

**⏳ Heti seuraavaksi:**
- [ ] Asenna MSVC Build Tools + Windows SDK (Windows-linkitystä varten — `cargo test --workspace` lokaalisti)
- [ ] Vaihe 4: E2EE-integraatio (prekey-vaihto → session → salaus → relay → purku desktopissa)

---

*Whisper — "Mä hoidan." 🔒*
