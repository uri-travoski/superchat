# SuperChat v1.1.3

A mobile-native UX pass on top of 1.1.2, driven by owner feedback. The server API is unchanged; 1.0 to 1.1.2 servers and apps keep working (`min_client_version` is still 1.0.0).

## What changed

* **Back works like a phone app.** One Back stack: popup, sheet or dialog first, then page, then chat, then list, and only then the app goes to the background. Esc and Alt+Left behave the same on desktop.
* **Popups are native.** Bottom sheets for menus and pickers, centred alert dialogs only for confirmations, full-screen pages for Settings, Search, New chat, Threads, Drafts, Chat info.
* **Chat list and chat screen are decluttered** (see CHANGELOG): 3-dot menu, floating New chat button, search page, chat info page, pinned/bookmark banner, "Aa" formatting toggle, long-press message actions.
* **Settings** is a grouped, icon-led, card layout with sub-pages and sheet pickers.
* **Android QR login works** (and can read a picture of the QR code); failures are shown as text.
* **Admin panel dialogs** all have a close X, Esc, backdrop click and fit small phones.

## Honest limitations

* The live camera scan was verified only up to the permission prompt on the emulator (its virtual camera shows a synthetic scene, not a QR code). The decode path was verified with a real QR picture through the photo picker, and a login link via `am start`. Please try the live camera once on a real phone.
* Swipe-down to close a sheet works from the drag handle and title area, not from the sheet body (so lists inside a sheet can scroll). Reply-by-swipe, pull to refresh and bottom navigation were not built (optional or not wanted).
* New group (admin) is not in the app; New channel is. Notification sounds and per-channel defaults are Android's own settings or the per-conversation Notify me about; there is no in-app sound picker. No storage/cache page and no in-app update check: SuperChat has no cache-clearing or update API to call.
* Predictive back is deliberately off (legacy Back dispatch, `enableOnBackInvokedCallback=false`) so the Rust Back stack and the system animation cannot disagree.
* The desktop app's chat history arrows were removed from the header; Alt+Left goes back through the stack and then through conversation history.
* Message rows keep the existing Slack-like layout (avatar, name, text); chat bubbles were not introduced.
* Windows and arm64 builds are still not executed.
