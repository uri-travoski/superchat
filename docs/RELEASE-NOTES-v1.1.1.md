# SuperChat v1.1.1

A small release on top of 1.1. 1.0 and 1.1 servers and apps keep working with it (`min_client_version` is still 1.0.0).

## New

* **Role management.** In the admin panel, Users, the owner now has a role menu (Member, Admin, Owner) for every other person, on the desktop table and on the phone cards. Changing a role asks for confirmation.
  The change takes effect at once: the person stays signed in, their admin rights appear or disappear on the next request, and their apps refresh their profile by themselves, so controls such as creating channels show or hide without a restart.
* **Rules.** Only an owner changes roles (admins see them but cannot); nobody changes their own role; at least one owner always remains, so to hand over ownership make the other person an owner first and then let them step you down;
  bots always stay members; disabled people can still be changed. Clear error messages explain each refusal.
* **Privacy options.** The Users table has a "Privacy options" menu with Allow and Disallow instead of a tick box; it saves as soon as you change it. Allow shows the Privacy screen in that person's app Settings.
* **Fixed:** the desktop app did not treat an owner as an admin when pinning messages and curating channels.

API: `PUT /api/v1/admin/users/{id}/role`, see [API.md](API.md). Admin guide: [ADMIN.md](ADMIN.md).

## Verification

`cargo test --workspace` green (new: role rule unit test and 5 role integration tests covering owner, admin, self, last owner, immediate session effect, bots, disabled users, backup and restore); `tests/e2e/run.sh` green;
Playwright admin-smoke, admin-viewer, admin-automation, admin-mobile and the new admin-roles (desktop and 390x844) green; clippy 0 errors.
