# SuperChat v1.0

First release. Server, admin panel, Linux/Windows/Android apps, CLI.

## Included (feature IDs)

* **Server**: SV-01..SV-17, SV-19..SV-22, SV-24. Accounts (admin-created, one-time login links, disable = immediate sign-out and local wipe), private channels/groups,
  DMs and notes on demand, edit rule, tombstones, reactions, threads, pins, bookmarks, user groups, preferences, drafts, search with filters, files in S3/R2/B2/MinIO/Google Drive
  (move and verify), per-user visibility with mandatory backup before clearing history, encrypted full backups with restore, content-free push (ntfy, FCM, APNs),
  bots (Telegram-style and native API), incoming and outgoing signed webhooks, forms and App Home, scheduled messages, optional AI search and recap.
* **Admin panel**: overview, users, conversations, chat viewer with bulk delete, storage, sessions, backups and restore, bots, webhooks, AI, groups, bookmarks.
* **Apps (desktop and Android share one UI)**: X-01..X-08, M-01..M-13, M-16, M-20..M-29, M-32, C-01..C-03, C-07, NV-01..NV-09, SR-01..SR-07, P-01..P-04, E-01..E-04,
  E-06 (Android share target), E-08 (tray), WF-01, A-01..A-05 (rendering of forms, APP tag, Home tab), F-01..F-03, N-01..N-03.
* **Packaging**: Linux AppImage and .deb, Windows portable .exe and installer, Android APK (arm64-v8a and x86_64), server image on ghcr.io.

## Not included

* Calls (SV-18, CL-01): planned for v1.1.
* End-to-end encryption (by decision): the server and admins can read messages.

## Known limitations (honest list)

* Google Drive storage, FCM, APNs, ntfy and the AI providers were only tested against local fakes, never the real services.
* Windows: the code paths (Credential Manager, tray, file chooser, notifications, installer) are compiled and packaged only in CI and were not run on a Windows machine.
  Treat the Windows build as untested until you try it.
* Android: arm64 compiles but was never executed (emulator is x86_64). Voice recording needs the microphone permission and was not exercised with real audio
  (the emulator has no audio input); the ntfy listener and Keystore were exercised on the emulator only as far as the logs show (see STATUS.md). FCM is not in the default APK.
* The default APK from CI is signed with a throw-away key unless you set the signing secrets; such an APK cannot update an install signed with another key.
* Emoji typed inside message text are drawn with the system font and may be monochrome; the picker and reactions use bundled colour pictures.
* Video clips and voice messages upload and play (audio in-app, video in the system player); there is no waveform, speed control or screen recording.
* Drag-and-drop of files works where winit delivers drops (X11, Windows), not on Wayland. Image paste is a button.
* Window and notification behaviour on a real desktop (IME, clipboard, tray hosts such as GNOME without an extension) was tested only on a private Xvfb.
* The SQLite cache is not encrypted (SQLCipher not done). Auto-away (P-02) is not built.
* Flatpak: a manifest is in `packaging/flatpak` but is not built in CI.
* Backup passphrase loss means encrypted backups cannot be read.
