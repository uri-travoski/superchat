# Navigation audit: every screen has a way back (1.3.0)

Rule: every page, panel, tab, modal and menu has a **visible** close or back control and also answers Esc (desktop) and the Android system Back key.
Messages is always one click or tap away. There is one Back stack, `apps/desktop/src/nav.rs`; Esc, Alt+Left, the on-screen arrows and Android Back all call it.

## How Back unwinds (top layer first)

1. Alert dialog (confirm), TOFU fingerprint dialog, form, schedule, bookmark and new-channel dialogs.
2. People picker and label picker.
3. Any sheet or popover menu (message, conversation, emoji, attach, sort, status, help, ...).
4. App Lock entry, attach bar, "jump" bar.
5. Settings sub-page, then Settings (or Search, Threads, Drafts, Switcher, People).
6. Details sub-page (phone: back to the Details list), then Details (phone page or desktop modal).
7. Thread (side panel or full screen).
8. Conversation tab (Files, Pins, Bookmarks) back to Messages (desktop).
9. The conversation itself (phone: back to the tab it was opened from).
10. Phone main tab (DMs, Activity, You) back to Home. Home then leaves the app (Android moves the task to the background).

The unit tests in `apps/desktop/src/nav.rs` (`cargo test -p atomic-desktop nav`) cover each of these orderings, including
chat > details > Files > back > details > back > chat, picker over details, sheet over settings, tabs returning to Home, and "every screen name has a layer that Back closes".

## Phone (360x640, bottom tabs Home, DMs, Activity, You)

| Screen | How to open | Visible way back | Back key / Esc goes to |
|---|---|---|---|
| Home | launch, Home tab | none needed (root) | leaves the app |
| DMs, Activity, You tab | bottom bar | tap Home in the bar | Home |
| Conversation | tap a row | top-bar back arrow | the tab list it was opened from |
| Conversation details | tap the name, the info icon or the 3-dot menu | back arrow | the chat |
| Details > About, Members, Files, Pins, Bookmarks, Manage (rename, archive, delete; admins), Notifications | rows on the Details page | back arrow | the Details list, then the chat |
| People picker (Add people, New message) | Members > Add people, pencil button | Cancel and X | the page below |
| Label picker | message sheet > Add label | X, tap outside | the page below |
| Message sheet | long-press a message | tap outside | the chat |
| Thread (full screen) | "N replies" row or sheet > Reply in thread | back arrow | the chat |
| Threads list | Home > Threads | back arrow | Home |
| Drafts & sent | Home > Drafts & sent | back arrow | Home |
| Search | Home search pill | back arrow, X | Home |
| Later (Saved items) | You > Saved items, Home > Later | back arrow or Home tab | You or Home |
| Settings and sub-pages | You > Preferences | back arrow | You (sub-page first) |
| App Lock page | Settings > Privacy & Security > App Lock, or "Set up App Lock" anywhere | back arrow | Settings |
| Unlock screen | padlock when locked | Cancel | Home |
| Forms, schedule, bookmark dialogs | their buttons | Cancel, X | the chat |

## Desktop (wide window: rail, sidebar, conversation, thread panel)

| Screen | How to open | Visible way back | Esc / Alt+Left |
|---|---|---|---|
| Home, DMs, Activity, Later, More | rail buttons | the other rail buttons | n/a (rail, not a stack) |
| Conversation | sidebar row | n/a | Alt+Left returns to the previous conversation |
| Files, Pins, Bookmarks tab | tab bar, details modal tabs | the Messages tab | Esc returns to Messages |
| Thread panel | "N replies", hover toolbar | X in the panel header | Esc closes it |
| Channel details modal | header name, chevron, member avatars, info icon, row menu > View details | X in the header, Esc, click outside | closes it (also over a thread) |
| Add people / Remove | Members tab | Cancel, X | closes the picker |
| Search overlay and results | top-bar search, Ctrl+K, Ctrl+F | arrow back, X | closes search |
| Preferences, App Lock | avatar menu, More, server menu | arrow back | closes it |
| Quick switcher | Ctrl+K | X | closes it |
| Help (`?`) | top bar | X | closes it |
| Menus (right-click, hover "...", avatar, server name) | pointer | click elsewhere | closes it |

## Findings from the audit and what was changed

* **Chat 3-dot menu > Files had no way back** (1.1.3 showed Files as a sub-screen of the details page with only a hidden gesture). Fixed: on the phone Files, Pins, Bookmarks, Members and About are pages of the Details screen with a back arrow; Back returns to Details and then to the chat. On desktop the tabs have a visible Messages tab and Esc.
* Details used to be a different screen per entry point; there is now one Details component reached from the title, the info icon, the member avatars and the conversation menu.
* The people picker, label picker, help and invite dialogs are layers of the same stack, so Esc and Android Back close them.
* On the phone each bottom tab returns to Home before the app is left (it used to leave the app straight away from any tab).
* A conversation opened from Activity, Later or Search returns to where it was opened from (the tab keeps its own list).

## Emulator Back-key runs

Run on `emulator-5556` (Regular and font scale 1.3): chat > 3-dot menu > Details > Files > Back (Details) > Back (chat) > Back (list); Settings > Privacy > App Lock > Back x3; thread > Back; Activity tab > Back (Home) > Back (leaves the app). Results are recorded in `STATUS.md` and screenshots in `docs/screenshots/android-1.2-*.png`.
