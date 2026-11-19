# Navigation audit: every screen has a way back (1.4.0)

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
10. Phone: a page inside a tab (a conversation, Settings, Activity, the You page, Later, Threads ...) goes back inside its own tab. Every tab keeps its own back stack (`PhoneNav` in `nav.rs`).
11. Phone main tab (Channels, Search, Menu) back to DMs. DMs then leaves the app (Android moves the task to the background).

The unit tests in `apps/desktop/src/nav.rs` (`cargo test -p atomic-desktop nav`) cover each of these orderings, including
chat > details > Files > back > details > back > chat, picker over details, sheet over settings, tabs returning to DMs, a separate back stack per tab, and "every screen name has a layer that Back closes".

## Phone (360x640, bottom tabs DMs, Channels, Search, Menu)

| Screen | How to open | Visible way back | Back key / Esc goes to |
|---|---|---|---|
| DMs (default, last tab is remembered) | launch, DMs tab | none needed (root) | leaves the app |
| Channels, Search, Menu tab | bottom bar | tap DMs in the bar | DMs |
| Activity, Settings, You page | Menu tab rows; Menu > avatar card opens the You page | back arrow | the Menu tab |
| 3-dots menu (Threads, Drafts & sent, Later, Sort, Filter) | top bar, far right | tap outside | the list |
| Conversation | tap a row | top-bar back arrow | the tab list it was opened from |
| Conversation details | tap the name, the info icon or the 3-dot menu | back arrow | the chat |
| Details > About, Members, Files, Pins, Bookmarks, Manage (rename, archive, delete; admins), Notifications | rows on the Details page | back arrow | the Details list, then the chat |
| People picker (Add people, New message) | Members > Add people, the + button on DMs and Channels | Cancel and X | the page below |
| Label picker | message sheet > Add label | X, tap outside | the page below |
| Message sheet | long-press a message | tap outside | the chat |
| Thread (full screen) | "N replies" row or sheet > Reply in thread | back arrow | the chat |
| Threads list | 3-dots menu > Threads | back arrow | the list it was opened from |
| Drafts & sent | 3-dots menu > Drafts & sent | back arrow | the list it was opened from |
| Search | Search tab (full page, field focused) | the DMs tab in the bar | DMs |
| Later (Saved items) | You page > Saved items, 3-dots menu > Later | back arrow | You page or the list |
| Settings and sub-pages | Menu tab > Settings | back arrow | the Menu tab (sub-page first) |
| About (version, Check for updates) | Settings > About, the "Update available" banner on DMs, Menu tab > Settings > About | back arrow | Settings, then where you came from |
| App Lock page | Settings > Privacy & Security > App Lock, or "Set up App Lock" anywhere | back arrow | Settings |
| Unlock screen | padlock when locked | Cancel | DMs |
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
| About (version, Check for updates) | Preferences > About, the rail "Update" button, Help > About SuperChat | arrow back | back to Preferences, then closes |
| Quick switcher | Ctrl+K | X | closes it |
| Help (`?`) | top bar | X | closes it |
| Menus (right-click, hover "...", avatar, server name) | pointer | click elsewhere | closes it |

## Findings from the audit and what was changed

* **Chat 3-dot menu > Files had no way back** (1.1.3 showed Files as a sub-screen of the details page with only a hidden gesture). Fixed: on the phone Files, Pins, Bookmarks, Members and About are pages of the Details screen with a back arrow; Back returns to Details and then to the chat. On desktop the tabs have a visible Messages tab and Esc.
* Details used to be a different screen per entry point; there is now one Details component reached from the title, the info icon, the member avatars and the conversation menu.
* The people picker, label picker, help and invite dialogs are layers of the same stack, so Esc and Android Back close them.
* On the phone each bottom tab returns to DMs before the app is left (it used to leave the app straight away from any tab).
* A conversation opened from Activity, Later or Search returns to where it was opened from (the tab keeps its own list).

## Emulator Back-key runs

Run on `emulator-5556` (Regular and font scale 1.3): chat > 3-dot menu > Details > Files > Back (Details) > Back (chat) > Back (list); Settings > Privacy > App Lock > Back x3; thread > Back; Activity tab > Back (Home) > Back (leaves the app). Results are recorded in `STATUS.md` and screenshots in `docs/screenshots/android-1.2-*.png`.
