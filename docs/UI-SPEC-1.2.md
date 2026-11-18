# UI specification 1.2 (Slack-style navigation, SuperChat theme)

Version 1.2.0 rebuilds the desktop and phone UI so that it works and feels like Slack (layout, navigation, interactions,
information architecture, discoverability) while keeping SuperChat's own look: Element X-like colours, rounded shapes, our
icon set, our name and icon. No Slack logo, name, illustrations or assets are used. One Slint UI serves desktop and Android;
the layout switches on window width (>= 720 px is "wide": rail + sidebar + main pane, otherwise "phone": bottom tabs).

## 1. Information architecture

| Area | Desktop (wide) | Phone |
|---|---|---|
| Home | Sidebar (Threads, Drafts & sent, Later, then Favourites, Channels, Direct messages, custom sections) + conversation | Home tab: same sections as a list, search pill, pencil button |
| DMs | DMs tab: list of direct messages and group chats with last message + conversation | DMs tab |
| Activity | Activity tab: filter chips All, @Mentions, Threads, Reactions; click opens the message in context | Activity tab, same chips |
| Later | Later tab: saved messages, label chips, filter by label | You > Saved items (Later) |
| More | Files, Browse channels, Drafts & sent, Threads, Preferences, App Lock | You tab |
| Profile | Avatar at the bottom of the rail: status, pause notifications, preferences, sign out (only when allowed) | You tab header |
| Search | Top bar search field > overlay (recent, filters) > results page with tabs | Search pill > full-screen search |

## 2. Desktop shell

```
+------+--------------------------------------------------------------+
| rail | top bar:  <  >     [ Search <server name>        ]        ?   |
| Home +-----------+--------------------------------------+-----------+
| DMs  | sidebar   | conversation header (# name v) avatars| thread    |
| Act. |  Threads  | tabs: Messages Files Pins Bookmarks   | panel     |
| Later|  Drafts   | message list (flat rows, hover bar)  | (resize)  |
| More |  Channels | composer (toolbar, + emoji @ send v)  |           |
| (me) |  DMs      |                                       |           |
+------+-----------+--------------------------------------+-----------+
```

* **Rail** (64 px): server initials tile at the top (opens the server menu), tabs Home, DMs, Activity (badge), Later, More; at the bottom the user avatar with a presence dot, which opens the profile menu: Set status (Online / Away / Do not disturb), Pause notifications (do-not-disturb toggle), Preferences, Sign out (only when allowed).
* **Top bar**: back and forward arrows (conversation history, Alt+Left/Right), a centred search field that reads "Search <server name>" and opens the search overlay, a help `?` button that lists the keyboard shortcuts.
* **Sidebar header**: server name with a chevron. Menu: Preferences; for admins also Open admin panel (copies the link), Create channel, Invite people (explains: create the user in the admin panel, then share the login link).
* **Sidebar sections**: Threads, Drafts & sent, Later (links with counts), Favourites, Channels (+ menu: Create channel for admins, Browse channels), Direct messages (+ New message), custom sections. Sections collapse; unread names are bold, mention badges are red, muted conversations are dimmed, locked conversations are absent while App Lock is engaged.
* **Row context menu** (right click, or the hover "..." button): Copy link, Mute, Mark as read / unread, Star (favourite), Move to section, Lock conversation (only with a PIN) or "Set up App Lock to lock conversations", View details.
* **Conversation header**: `#` or lock icon + name + chevron (opens Channel details), member avatar stack with a count (opens details on Members), a details icon. Tab bar: Messages, Files, Pins, Bookmarks (+ Add a bookmark for admins). Esc returns to Messages.
* **Messages**: flat rows (no bubbles). Avatar + bold name + time on the first row of a group, compact continuation rows, day dividers with a date pill, "New messages" divider. A row highlights on hover and shows a floating toolbar at its top right: three quick reactions, Add reaction, Reply in thread, Forward, Save for later (toggles), More (Copy link, Pin, Edit, Delete, Mark unread, Add label...). Reaction pills under the text (highlighted when mine), thread summary row (reply avatars, "N replies", last reply time), "(edited)", APP tag, form cards, file cards, inline images. Editing happens in place (Enter saves, Esc cancels). System messages ("Alice added Bob") are centred and muted.
* **Thread panel** (right, resizable by dragging its left edge, 320 to 560 px): parent, replies, composer with "Also send to #channel", close X, Esc closes. The sidebar Threads view lists followed threads with unread state.
* **Channel details** (modal, tabs About, Members, Settings, Files, Pins): About shows topic/description, kind, created date (admins edit topic); Members lists people with an "Add people" button and a remove action for owners and admins; Settings has notification level, mute, Lock conversation / Set up App Lock; Files and Pins list items. A visible X closes the modal (also Esc and the backdrop).
* **Composer**: rounded box; formatting row inside it (B, I, S, link, ordered list, bulleted list, quote, code, code block); bottom row: + (attach file, paste image, voice message, schedule send), emoji, @ mention, then the Send button with a chevron that opens "Schedule message". Enter sends, Shift+Enter inserts a newline, Up in an empty box edits your last message, drafts are kept per conversation, @name and :emoji autocomplete popovers.
* **Activity view**: rows for mentions, reactions and thread replies with the actor, a snippet, the channel and the time; unread rows are bold with a dot. "Mark all read".
* **Later view**: saved messages as cards with their label chips; chips above filter by label; "Manage labels" creates, renames, recolours and deletes labels. Empty state explains how to save.
* **DMs view**: list with last message and unread state; "New message" opens the people picker.
* **Search**: overlay with recent searches and filter shortcuts (from:, in:, has:, before:, after:), then a results page with tabs Messages, Files, People, Channels, filter dropdowns and highlighted matches; a click jumps to the message in context.
* **Keyboard**: Ctrl+K quick switcher, Ctrl+F search, Alt+Up/Down previous/next conversation, Alt+Left/Right history, Esc closes the top layer, Ctrl+Shift+A opens Activity, Ctrl+, opens Preferences.

## 3. Phone shell

* **Bottom navigation** (4 tabs: Home, DMs, Activity, You; 56 dp bar, labels visible, Activity badge). Each tab keeps its own Back stack: Back pops inside the tab, then returns to Home, then leaves the app. The bar is hidden on full-screen pages (conversation, thread, details, search, pickers).
* **Home**: top app bar with the server avatar and name, an App Lock padlock only when a PIN is set, and a search pill "Jump to or search...". Collapsible sections Threads, Drafts & sent, Favourites, Channels, Direct messages, custom sections. A round pencil button (bottom right) opens New message (people and channel picker).
* **Conversation screen**: top bar with back arrow, name (tap opens Details), a subtitle (members or presence), an info icon. Flat message rows; long press opens a bottom sheet (quick reactions, Reply in thread, Copy text, Copy link, Save for later, Add label, Pin, Edit, Delete, Mark unread, Forward). Thread summary rows. Composer: +, text field, emoji, send; formatting toggled with "Aa".
* **Details screen** (from the title or the info icon): header (name, topic, member count) and rows About, Members (Add people for owners/admins), Files, Pins, Bookmarks, Notifications, Mute, Lock conversation (or Set up App Lock). Every row opens a full page with a back arrow that returns to Details; Back again returns to the chat.
* **Threads**: a Threads screen (from the Home section) lists followed threads as cards (channel, parent snippet, latest replies, unread); a full-screen thread view shows parent, replies and a composer with "Also send to channel"; Back returns to where you came from.
* **Activity**: filter chips All, Mentions, Threads, Reactions; rows open the message in context.
* **You**: avatar, name, status; Set status, Pause notifications, Saved items (Later) with label filter, Preferences (the 1.1.3 settings pages), Devices and Sign out (only when allowed), App Lock.

## 4. App Lock discoverability

Settings > Privacy & Security > App Lock explains in one line "Locked conversations are hidden until you unlock with your PIN", lists the locked conversations with a "+" to add one and an x to remove one. A conversation's context menu (right click, long press) and its details page offer "Lock conversation" when a PIN is set and "Set up App Lock to lock conversations" (which opens the App Lock setup) when not. The padlock in the top bar locks or unlocks. The unlock screen stays deliberately generic ("App Lock", 6-digit PIN, Cancel).

## 5. Saved items and labels

Per user, private, synced. Save for later is a bookmark icon in the hover toolbar and in the message sheet; "Add label..." opens a small label picker (checkbox list, "New label" row with colour). Later shows saved messages newest first; a deleted message shows "Message deleted"; messages in locked conversations are hidden while locked.

## 6. Navigation map and the way back

See [UX-NAVIGATION.md](UX-NAVIGATION.md) for every path and how to go back. Rule: every page, panel, tab and modal has a visible close or back control and answers Esc / Android Back; Messages is always one click away.

## 7. Components

`Rail`, `TopBar`, `Sidebar`, `SidebarItem`, `Tabs`, `Modal`, `Sheet` (bottom sheet / popover), `MessageRow`, `HoverToolbar`, `ReactionPill`, `ThreadSummary`, `Composer`, `PeoplePicker`, `LabelChip`, `EmptyState`, `Avatar`, `SettingRow`. State lives in one Slint global (`App`), data in Rust (`apps/desktop/src`). Tokens (colours, spacing 4/8/12/16/24, radii, type scale, 48 dp touch targets, contrast >= 4.5:1) are in `ui/theme.slint`. Regular and Large text scale without clipping; light and dark themes.

## 8. Interaction states

Hover (rows, buttons) raises the background; pressed darkens; selected rows use the accent tint; disabled uses muted ink; focus is visible on text fields; empty states always say what the list is for and offer the action (no saved items, no threads, no activity, no files, no pins, no results).
