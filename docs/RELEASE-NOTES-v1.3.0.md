# SuperChat v1.3.0

Owner fixes to the 1.2 phone and desktop UI, and every feature that was still "partial" in the 1.2.0 audit except calls. The server API is additive (`PUT /files/{id}/meta`; deleting a channel now also removes its pins, saved items and files), so 1.0 to 1.2 apps keep working against a 1.3 server; a 1.3 app against an older server was not tested.

## What changed

* **Menus and popups**: on desktop every menu opens as a popover next to what was clicked (attach, emoji, row menus, server and avatar menus, the sound menu); on phones they stay bottom sheets. Every menu was looked at in a screenshot and the clipped ones were fixed. The search filters "From" and "In" were empty in 1.2 (their lists were never filled); fixed.
* **Uploads**: progress bar with bytes sent, Cancel, and Retry that keeps the files that already finished.
* **Voice and video**: voice messages show a waveform and duration and have play, pause, seek and 1x/1.5x/2x speed. Video clips carry a poster frame and duration. Android records inside the app (CameraX, 480p) after showing the server's upload limit, shows the size while recording and stops just under the limit; it also has a photo and video picker and a file picker (before, Android could only receive files through Share). Desktop has Upload video and Record screen (needs `ffmpeg` on X11 or Windows).
* **PDF preview**: the server draws the first page (pure Rust, no poppler in the image) and clients show it as a card; a broken PDF falls back to a plain file card.
* **Profile photos** show everywhere a person appears. Android picks the photo with the system picker (it only had a box for a file path before); desktop opens the file dialog.
* **Delete a channel** from Details with the name typed to confirm. On phones Details has a new Manage row (rename, archive, delete); before, those admin tools could not be reached on a phone.
* **Notification sounds**: bundled sounds, a default and a choice per conversation, a global switch, an unread number on the badge. Android uses one notification channel per sound.
* **Tray icon** (desktop): unread dot and count, a Status and Pause notifications menu, start minimised to the tray.
* **Sidebar order**: drag and drop (desktop) or Move up/down (both) for conversations and sections, synced between your devices.
* **Spell check** (desktop): red squiggles in the message box, right-click suggestions, Add to dictionary, Ignore, a language choice (bundled en_AU plus Hunspell dictionaries found on the system). Android uses the keyboard's own spell check.
* **Encrypted local cache**: the app's cache is SQLCipher-encrypted with a key kept in the secret store; an older plain cache is converted on first start. **Windows Credential Manager** holds secrets on Windows.
* **Push**: ntfy wake-ups were driven end to end on the Android emulator against a real ntfy container (`tests/e2e/ntfy-emulator.sh`); `docker compose --profile ntfy` and its ports are configurable. See `docs/DEPLOY.md`.
* **Paste**: Ctrl+V of files and images into the message box (desktop), "Paste image or file from clipboard" in the attach menu.
* **Licences**: `LICENSE` and `THIRD-PARTY-NOTICES.txt` (Inter, Noto emoji, en_AU dictionary) ship in the deb, AppImage, Flatpak and Windows installer.
* Version 1.3.0 everywhere; Android versionName 1.3.0 / versionCode 7; Docker image `superchat:1.3.0`.

## Honest limitations

* Calls (CL-01, SV-18) are still not built.
* **Windows** was never run here: the Credential Manager code and the installer are type-checked or scripted only; a CI job runs the secret round trip on a Windows runner, so treat Windows as unproven until that job has passed on a tag.
* **Android camera and pickers** were driven only on the x86_64 emulator's virtual camera. Real phone cameras, vendor photo pickers and the keyboard's spell check were not tried. The APK was run on the emulator only, never on a real phone, and the arm64 libraries were compiled but not run.
* **Push**: ntfy was verified on the emulator; Google FCM compiles only with a Firebase config and has never run on a device; APNs is not built.
* **Desktop**: drag and drop of files was tried on X11 only (Wayland could not be tested here). The tray was tested against a fake StatusNotifierWatcher, not a real panel; GNOME needs its AppIndicator extension. Screen recording needs `ffmpeg`. Screenshots come from a private Xvfb session without a window manager, so real-desktop IME and clipboard behaviour are untested.
* **Cache encryption** covers the database only, not the cached attachment files; App Lock is still a screen lock, not encryption.
* Sounds are per device and not synced.
* One unexplained one-off on the emulator: after upgrading a debug build in place, the previous encrypted cache was moved aside as `cache.db.unreadable` and a new cache was started (it is only a cache; it was rebuilt from the server and an in-place upgrade done afterwards kept the encrypted cache). I could not reproduce it.
