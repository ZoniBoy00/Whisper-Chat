; Whisper NSIS installer hooks (bundled via tauri.conf > nsis.installerHooks)
;
; The uninstaller removes the application files, but by default Tauri leaves
; the per-user data folder (%APPDATA%\com.whisper.desktop) behind. Whisper's
; local data is security-sensitive: it holds the E2EE identity keys and the
; full message history. Ask the user explicitly before wiping it, and only
; delete it after a confirmed uninstall.

!macro NSIS_HOOK_PREUNINSTALL
  MessageBox MB_YESNO|MB_ICONSTOP|MB_DEFBUTTON2 "Uninstall Whisper and permanently delete ALL local data?$\r$\n$\r$\nThis removes your encryption identity, every message, contact and setting from this computer. This cannot be undone.$\r$\n$\r$\nPoistetaanko Whisper ja KAIKKI paikallinen data?$\r$\nTämä poistaa salausavaimet (identity), viestihistorian, yhteystiedot ja asetukset. Tätä ei voi kumota." IDYES +2
  Abort
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  RMDir /r "$APPDATA\com.whisper.desktop"
!macroend
