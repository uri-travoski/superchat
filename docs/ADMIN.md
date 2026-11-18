# Admin guide

Open `https://your-server/admin` and sign in with an owner or admin account.

* **Users**: create (optionally with a password; a one-time login link is always produced), disable (the person is signed out
  of every device immediately and their app wipes local data), enable, reset password (signs out all devices; produces a link if
  no password is typed), login link, back up, **clear history**, delete.
* **One-time login link**: `superchat://login?server=<url>&token=<token>`. The person pastes it on the app's sign-in page.
  It works once and expires after 7 days. The app then asks them to choose a password.
* **Clear history** always writes a per-user backup first and refuses to proceed if the backup fails. It removes only that
  person's copy of every chat (other participants keep theirs), moves files nobody else can see into
  `backups/users/<user>/<backup>/att/` in your storage, and signs the person out everywhere.
* **Conversations**: channels and group DMs. All channels are private; only admins create them and manage members.
* **Storage**: choose backend (local, S3-compatible incl. R2 and B2, Google Drive), Test, Save (only before any file exists),
  **Move all files** to another backend (copy, verify size and checksum, switch, delete old copies; uploads pause meanwhile),
  Verify (quick or deep).
* **Sessions**: every signed-in device with last seen; sign any out.
* **Backups**: list of per-user backups.
* **Privacy screen** per user: when on, the user may hide their last-active time from others. Default off.

The same operations exist in the `atomic admin ...` CLI (`atomic help`).

## Phase 2: chat viewer, bulk delete, backups

* **Chats** (viewer): pick a user, then a channel, group or DM from the dropdowns; the timeline looks like a chat app (inline pictures,
  reactions, thread reply counts). Search with a date range inside the conversation or across everything. An admin can post in conversations
  they are a member of.
* **Bulk delete**: select messages (or a date range), choose "for everyone" or "only this person's copy" (plus the other participant's copy if wanted).
  A backup is written first; if it fails nothing is deleted. Files that nobody can see any more move into the backup folder.
* **Backups**: back up now, set the schedule and target, download, and restore. Restore a single person (merge keeps what they have now, replace makes
  their chats match the backup), a message backup (put deleted messages and files back), or everything. On a brand new server the setup page offers
  "restore from backup" before creating an owner.
* The backup passphrase lives in `<data>/backup.key` (created on first start) or in `ATOMIC_BACKUP_PASSPHRASE`. It is never stored inside a backup. Copy it
  somewhere safe: without it the backups cannot be read.
* **Push**: Settings > Push holds the ntfy server, FCM service account and APNs key (masked once saved).

## Automation pages

* **Bots**: create a bot (username, name, description, conversations to add it to); the token is shown once on creation and can be shown or regenerated later
  (the old one stops at once). Membership: add or remove the bot per conversation. The card shows its webhook (URL, secret set, last error) or the
  number of updates waiting for polling. See [BOTS.md](BOTS.md).
* **Webhooks**: incoming (a secret URL per conversation; "New URL" rotates it) and outgoing (URL, secret, event filter, per-conversation filter, status, "Send test").
* **AI**: provider, model, base URL and key. Off by default; the page repeats that message text goes to the provider.
* **Groups**: `@handle` groups (members, rename, delete). Typing `@handle` mentions everyone in the group.
* **Bookmarks**: pick a conversation and add, edit or remove its link bookmarks (admins can edit any conversation).
* **Chats**: the viewer marks bot messages with APP, draws forms as cards, and lists scheduled messages of the open conversation.
