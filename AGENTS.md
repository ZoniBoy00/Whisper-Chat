# AGENTS.md — Whisper

Yhteinen kontekstidokumentti tekoälyavustajille (agentit), jotka työskentelevät
tässä repossa. Tekniset termit ovat englanniksi, selitteet suomeksi.

## Projektin kuvaus

Whisper (työnimi Operation Ghost) on privacy-first, end-to-end-salattu (E2EE)
chat — WhatsApp/Signal/Telegram -korvaaja.

- **`server/`** (crate `whisper-relay`): sokea välittäjä (blind relay). Se näkee
  vain salatut enveloppet (encrypted envelopes) — ei koskaan plaintextiä tai
  avaimia. Palvelimella ei ole pääsyä viestien sisältöön.
- **`e2ee-core/`**: jaettu kryptoydin (crypto core). Perustuu **vodozemac**-kirjastoon:
  X3DH-avainvaihto + Double Ratchet -viestisalaus. Kaikki salaustyö tapahtuu täällä,
  ei koskaan palvelimella.

Arkkitehtuurin ydinperiaate: **palvelin on zero-knowledge** — se voi välittää ja
tallentaa vain salattuja enveloppeja.

## Tekniset säännöt

1. Koodikommentit AINA **englannin** kielellä. Suomenkieliset kommentit eivät ole
   sallittuja (README- ja doksikieli on eri asia).
2. **Kryptoa EI kirjoiteta itse** — kaikki kryptografia tulee
   [vodozemac](https://github.com/matrix-org/vodozemac)-kirjastosta (X3DH,
   Double Ratchet). Omia kryptoprimitiivejä, omia protokollaversioita tai
   "parannuksia" olemassa oleviin algoritmeihin ei hyväksytä.
3. **TDD**: kaikki kryptomuutokset vaativat testit ennen mergeä. Testit kirjoitetaan
   ensin tai vähintään samassa commitissa kuin koodi.
4. **"Jos toimii, anna olla"** (if it works, leave it alone) — älä refaktoroi
   toimivaa koodia ilman erillistä syytä. Vältä turhia kosmeettisia muutoksia.
5. **Server-spesifejä tiedostoja ei stageata GitHubiin**: `server/data/` (runtime-tiedot)
   ja `.env` (salaisuudet). Näiden tulee olla `.gitignore`ssä eikä niitä koskaan
   lisätä repo-muutoksiin.
6. **Code hardening**: release-buildit aina workspace-profiililla
   (`lto="fat"`, `panic="abort"`, `strip=true` — määritetty juuren Cargo.toml:ssa).
   Älä lisää debug-symboleita, älä vaihda `panic="unwind"`ia tuotantoon, äläkä
   koskaan vie salaisuuksia tai avaimia UI/JS-kerrokseen.

## Pipeline-työnjako

Työ tehdään jäjestelmällisesti seuraavassa järjestyksessä:

1. **Planner** — suunnittelee tehtävän, pilkkoo sen osatehtäviin.
2. **Coder(s)** — toteuttavat koodia. Max **3 koodaajaa rinnakkain** (ei enempää,
   jotta ei tule konflikteja samaan koodikantaan).
3. **Tester** — ajaa testit ja varmistaa että muutokset toimivat.
4. **Reviewer** — tarkistaa koodin (suojaus, tyyli, säännöt).
5. **Reporter** — raportoi lopputuloksen käyttäjälle.
6. Käyttäjä **hyväksyy** tuloksen.
7. **Push** — muutokset viedään GitHubiin.

Kukin rooli odottaa edellisen valmistumista; rinnakkaista koodausta max 3.

## Komentojen pikaohje

```sh
# Aja koko workspacen testit
cargo test --workspace

# Lint + hylkää varoitukset
cargo clippy --workspace -- -D warnings

# Formatoinnin tarkistus
cargo fmt --check

# Smoke test (vaatii, että serveri on käynnissä eri terminaalissa):
cd server
cargo build
node tests/smoke.mjs
```
