# SuperChat v1.1.2

A small release on top of 1.1.1. 1.0 and 1.1.x servers and apps keep working with it (`min_client_version` is still 1.0.0).

## New

* **Sign out is a privilege.** "Sign out" and "Sign out of all devices" appear in the desktop and Android apps only for people whose Privacy options is Allow in the admin panel (admins and owners always have them).
  For everyone else (Disallow, the default) these buttons and the Devices list are hidden, so nobody ends their own session by accident. Only an admin (Users, Sessions), or the server (disable, password reset, clear history) signs them out.
* **Live.** Flip the setting and open apps show or hide the controls at once, without a restart.
* **Enforced on the server.** Logout, revoking your own sessions and revoking other sessions answer `forbidden` ("sign-out is not allowed for this account") for members without Allow, so a modified app cannot bypass it. Admins are never locked out.
* **Sign out of all devices** (`POST /sessions/revoke-all`, CLI `logout-all`) ends every session including this one, after a confirmation.
* Admin panel: the Privacy options menu and its help text explain what Allow controls.

Older 1.1.1 apps still show the buttons for Disallow users, but the server refuses them. Forced sign-outs (disabled, password reset, history cleared, revoked by an admin) are unchanged.

API: see [API.md](API.md). Admin guide: [ADMIN.md](ADMIN.md). Decisions: [DECISIONS.md](DECISIONS.md).

## Verification

`cargo test --workspace` green (159 tests; new `signout.rs`: Disallow refuses logout, logout-all, revoke-others and revoke of own sessions with sessions unaffected; Allow works and logout-all ends every device; admins exempt and admin-panel revocation works; live flip via `me_update`; forced wipes still work);
`tests/e2e/run.sh` green (the CLI check now also proves logout is refused without Allow); Playwright admin-viewer, admin-automation, admin-mobile and admin-roles green; clippy 0 errors;
screenshots of Settings without (`desk-*-settings-no-signout`) and with (`desk-*-settings-signout`) the sign-out controls, Regular and Large.
