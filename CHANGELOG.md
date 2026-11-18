# Changelog

## 1.1.1

* Role management: the owner can change a user's role (member, admin, owner) from the admin panel's Users page, with a confirmation. `PUT /api/v1/admin/users/{id}/role`. The change applies at once to the person's signed-in devices (no new login) and their apps show or hide admin-only controls without a restart.
* Rules: only an owner changes roles; nobody changes their own role; at least one owner always remains; bots stay members; disabled users can be changed.
* Fixed: the desktop app did not treat an owner as an admin for pinning and curating.
* Admin panel: the "Privacy screen" tick box is now a "Privacy options" menu (Allow / Disallow) that saves as soon as it changes.
* Compatible with 1.0 apps and servers (`min_client_version` is still 1.0.0; the existing `me_update` event carries the new role).

## 1.1.0

* Renamed Atomic Chat to SuperChat (compatible: `ATOMIC_*` variables, old data folders, old login links, docker volume keep working). New icon.
* Login links are web addresses (`<server>/l/<token>`) that open the app and sign the person in; QR code in the admin panel; "Scan QR code" and link intent filters on Android; `superchat://` handler on Linux and Windows.
* Multiple AI providers with defaults per feature, test, masked keys, failover.
* App Lock: local 6-digit PIN, locked conversations hidden until unlocked, generic notifications.
* Admin panel usable on phones; owner setting for the public address used in links.
* Login screen: Username & password / Login link buttons, Paste, link detection in any field, exact errors.
* Fixed: deleting a user who had signed in returned 500. Server errors are now logged.

## 1.0.0

First release. See docs/RELEASE-NOTES-v1.0.md.
