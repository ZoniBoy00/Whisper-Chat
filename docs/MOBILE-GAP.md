# Whisper Mobile — Gap Analysis & Task List

> Päivitetty: 2026-08-14 (commit 78e4a35)
> Status: Dokumentti OpenCodelle — mitä vielä pitää tehdä/korjata mobiilissa

## ✅ Jo tehty (ei enää listalla)

| Ominaisuus | Tila |
|---|---|
| Backup/restore (Argon2id 19MiB + AES-256-GCM, v2-formaatti, tamper-proof) | ✅ 5 unit-testiä |
| Safety numbers (`safetyNumber`, `shortSafetyNumber`) | ✅ |
| Avatar + username-rekisteröinti (`registerProfile`, `setAvatar`) | ✅ |
| Logs-tab | ✅ |
| Flutter-testit (l10n, teema) + CI "Mobile (Flutter)" -jobi | ✅ |
| mobile/README | ✅ |
| Ryhmät, reaktiot, edit/delete, receipts, i18n-runko, katoavien viestien UI | ✅ |

---

## 🔴 KRITTISET BUGIT

### B1. Mobiilin Whisper ID ei löydy desktopin hausta (ja päinvastoin)
- Mobiilin peer ID (esim. `317b8da4...`) ei löydy, kun sitä hakee desktop-versiossa
- Tarkistettava:
  - (a) rekisteröityykö profiili relaylle aina (registerProfile) — myös silloin kun usernamea ei ole asetettu
  - (b) tukeeko `search_users` peer-ID-prefix-hakua, ei vaan username-prefixiä
  - (c) toimiiko haku molempiin suuntiin: mobiili→desktop ja desktop→mobiili
- **Tämä on release-blocker-tason juttu** — kahden koneen E2EE-testi vaatii että osapuolet löytää toisensa

### B2. Asetuksissa scrollatessa kaikki vaihtaa kokoa
- Settings-näkymässä scrollaus aiheuttaa tekstien/elementtien koon vaihtelua
- Todennäköinen syy: Flutter-layout (MediaQuery.textScaler, epävakaa ListView/SingleChildScrollView, tai rebuild-skaalaus)
- Korjaus: lukitse fonttikoot, poista dynaaminen skaalaus, testaa eri näyttökoilla/DPI:llä

---

## 🔴 PUUTTUVAT OMINAISUUDET (desktop-pariteetti)

### P1. Oman nimen (username) vaihto
- `signUsername` on vaan rekisteröintiin. Desktopin "Change username" (update_profile-viesti) puuttuu mobiilista
- Kun nimi on kerran asetettu, sitä ei voi vaihtaa

### P2. Push-ilmoitukset (FCM/APNs)
- "Notification options are coming to mobile soon" -placeholder
- **Vaatii Firebase-projektin luonnin (Jonin toimi)** — OpenCode ei voi luoda Firebase-tiliä
- Vain "sinulla on viesti" -push (roadmap §8)

### P3. Katoavien viestien protokollakytkentä
- UI-dialogi on olemassa (main_screen), mutta **varmista että valinta (Off/5s/30s/1h...) oikeasti menee lähetettävään viestiin** (disappear-kenttä), ei ole vaan UI-koriste

### P4. Profiilinäkymä toisesta käyttäjästä
- `getProfile` on kutsuttavissa, mutta ei ole kunnon profiiliruutua (desktop: ProfileDialog — avatar, username, peer ID, safety numbers -linkki)

### P5. Ryhmän avatar
- Serveri tukee (`group_avatar_set`), mobiilissa ei UI:ta

---

## 🟡 UI/UX-PARANNUKSET

### U1. "Your Whisper ID" -header
- Vie tilaa päässä → siirrä asetuksiin (desktopin PeerIdCard-malli)

### U2. "Select a conversation" -oikea paneeli
- Desktop-jäänne → poista mobiilista (turha, vie tilaa)

### U3. Whisper ID -näyttö
- Näytä lyhennettynä + kopiointi-nappi (ei täyspitkää merkkijonoa joka vie tilaa)

### U4. Desktop-paletin tarkka match
- Vertaa theme.dart desktopin CSS:ään (värit, fontit, spacing, kuplat, avatarit)

### U5. i18n-täydellisyys
- Kieli vaihtuu, mutta käy läpi että KAIKKI tekstit on käännetty (desktopissa 1036 riviä käännöksiä vs mobiilin i18n.dart)

---

## 🔧 TEKNISET / LAATU

### T1. whisper_core dead-code-varoitukset
- 7 kpl (esim. `FriendRequestIncoming`-Debug) — siisti pois, clippy puhdas

### T2. Flutter-testien laajennus
- Vain 2 pientä testiä (l10n, theme). Lisää widget-testejä: onboarding, settings, chat-view, backup-encrypt/decrypt-roundtrip

---

## 💡 Suositusjärjestys

1. **B1 (peer-haku)** — release-blocker, tekee E2EE-testin mahdottomaksi
2. **B2 (scroll-skaalaus)** — näkyvä bugi jokaisessa käytössä
3. **P1 (username-vaihto)** — käyttäjän raportoima puute
4. **P3 (disappear-kytkentä)** — protokolla valmis, altista vaan
5. **P4 (profiilinäkymä)** + U1-U3 (UI-siivous)
6. **T1-T2** (laatuportti)
7. **P2 (push)** — isoin feature-jäännös, vaatii Firebase-projektin (Jonin toimi)
8. **P5 (ryhmän avatar)** + U4-U5 (viimeistely)
