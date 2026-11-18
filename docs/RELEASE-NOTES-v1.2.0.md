# SuperChat v1.2.0

A Slack-style rebuild of the desktop and phone UI on top of 1.1.3, driven by owner feedback ("very basic" desktop, no way to add people to channels, App Lock hard to find, no labels, no way back from Files, threads on mobile). SuperChat keeps its own theme, name, icon and icon set. The server API is additive; 1.0 to 1.1.x apps keep working (`min_client_version` is still 1.0.0).

## What changed

* **Desktop**: rail, top bar with search, sidebar with sections, flat message rows with a hover toolbar, thread panel, channel details modal with Members and Add people, conversation tabs, Activity, Later with labels, search with result tabs and filters, keyboard shortcuts (Ctrl+K, Ctrl+F, Alt+Up/Down, Esc, Ctrl+Shift+A).
* **Phone**: bottom navigation (Home, DMs, Activity, You) with a Back stack per tab, Home with search pill and New message button, long-press sheet, Details screen, full-screen threads, You tab.
* **Owners and admins** add and remove people in channels and groups, rename, edit the topic and archive from the app.
* **Save for later and labels**, a server-backed **Activity** feed, quote reply, `@channel`/`@here`/`@everyone`/group mentions, `:emoji` autocomplete, search highlighting, a global Files view, auto-away.
* **Polish pass**: the phone New message button is also in the top bar and the floating one hides while it would cover the last row; "N new messages" is a small floating pill (arrow up or down) that hides once the first unread is visible, and a slim bar under the tabs on desktop; search snippets are plain single-line text with the match highlighted; long settings titles shrink instead of truncating; the label picker has a proper checkbox column; sheets are full width at large text.
* **App Lock** is explained in Settings and lists locked conversations; conversation menus offer Lock conversation.
* **A way back from every screen**: see `docs/UX-NAVIGATION.md`.
* **Feature audit** of the original requirements: `docs/FEATURE-AUDIT.md` (130 rows).

## Honest limitations

* Calls (CL-01, SV-18) are still not built.
* Partial: voice messages have no waveform or speed control; video recording is upload only; no upload progress or cancel; SQLCipher is not built; section reorder by drag is not built (Move to section instead); other people's profile photos are initials; per-channel sounds; spell check is only a stored switch; tray icon is not created; push (FCM, ntfy, APNs) is tested with fakes only; delete channel is done in the admin panel (the app links to it).
* The phone UI was driven by hand on one emulator (x86_64, Regular and 1.3 font scale); the APK was never run on a real phone, and arm64 and Windows builds are still not executed. Auto-away was not watched over a real 10 minute idle.
* The desktop "Jump to first unread" bar is the same logic as the phone pill (checked on the emulator) but its appearance was not captured by the screenshot harness.
* Desktop screenshots come from a private Xvfb session without a window manager; real-desktop IME, clipboard and drag-drop are untested.
* App Lock is a screen lock, not encryption.
