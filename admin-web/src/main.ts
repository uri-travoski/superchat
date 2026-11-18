import "./style.css";
import { api, ApiError, login, meta, signedIn, signOutLocal, useLogin, username } from "./api";
import type { AdminConversation, AdminUser, BackupInfo, JobStatus, ServerMeta, SessionInfo, StorageConfig } from "./api-types";
import { copy, dialog, field, fmtBytes, fmtDate, h, toast } from "./dom";
import { storageForm } from "./storage-form";
import { chatsPage, leaveViewer } from "./viewer";
import { backupsPage, restoreSetupForm } from "./backups";
import { bookmarksPage, groupsPage } from "./orgadmin";
import { aiPage, botsPage, webhooksPage } from "./automation";
import { qrSvg } from "./qr";

const app = document.getElementById("app")!;
const tabs = ["overview", "chats", "users", "conversations", "groups", "bookmarks", "bots", "webhooks", "ai", "storage", "sessions", "backups"] as const;
type Tab = (typeof tabs)[number];

function tab(): Tab {
  const t = location.hash.replace("#/", "") as Tab;
  return tabs.includes(t) ? t : "overview";
}

function fail(e: unknown) {
  const err = e as ApiError;
  if (err?.code === "session_revoked" || err?.code === "account_disabled" || err?.code === "unauthorized") {
    signOutLocal();
    render();
  }
  toast(err?.message ?? String(e), true);
}

async function render() {
  let m: ServerMeta;
  try { m = await meta(); } catch (e) { app.replaceChildren(h("div", { class: "login card" }, h("h2", {}, "Cannot reach the server"), h("p", {}, String((e as Error).message)))); return; }
  if (m.setup_required) return renderSetup(m);
  if (!signedIn()) return renderLogin(m);
  leaveViewer();
  const body = h("main", {});
  const nav = h("nav", { class: "tabs", id: "nav" }, ...tabs.map((t) => h("a", { href: "#/" + t, class: t === tab() ? "on" : "" }, t === "ai" ? "AI" : t[0].toUpperCase() + t.slice(1))));
  const menu = h("button", { class: "menu-btn", id: "menu-btn", "aria-label": "Menu", "aria-expanded": "false", "aria-controls": "nav", onclick: () => { const o = nav.classList.toggle("open"); menu.setAttribute("aria-expanded", String(o)); } }, "☰ Menu");
  app.replaceChildren(
    h("header", { class: "top" }, menu, h("img", { class: "logo", src: "/admin/logo.png", alt: "", width: 28, height: 28 }), h("h1", {}, m.name + " admin"), h("span", { class: "muted" }, "v" + m.server_version), h("div", { class: "spacer" }),
      h("span", { class: "muted" }, username()),
      h("button", { onclick: async () => { try { await api("POST", "/logout"); } catch { /* ignore */ } signOutLocal(); render(); } }, "Sign out")),
    nav,
    body);
  try {
    await pages[tab()](body, m);
  } catch (e) { fail(e); }
}

window.addEventListener("hashchange", render);

// ------------------------------------------------------------------ setup & login

function renderSetup(m: ServerMeta) {
  const user = h("input", { value: "admin", autocomplete: "username" });
  const name = h("input", { value: "Administrator" });
  const email = h("input", { type: "email", placeholder: "optional" });
  const pw = h("input", { type: "password", autocomplete: "new-password", placeholder: "at least 8 characters" });
  const sf = storageForm();
  const status = h("div", { class: "muted" });
  app.replaceChildren(h("div", { class: "login card", style: "max-width:560px" },
    h("h2", {}, "Set up " + m.name),
    h("p", { class: "muted" }, "Create the owner account and choose where attachments are stored. You can move files to another storage later."),
    h("p", {}, h("a", { href: "#/restore", id: "restore-link", onclick: (e: Event) => { e.preventDefault(); renderRestore(m); } }, "Restoring a server? Restore from a backup instead.")),
    field("Username", user), field("Display name", name), field("Email", email), field("Password", pw),
    h("h3", {}, "Attachment storage"), sf.el, status,
    h("div", { class: "row", style: "margin-top:16px" },
      h("button", { onclick: async () => {
        status.textContent = "Testing...";
        try {
          const r = await api("POST", "/admin/storage/test", sf.value()).catch(() => null);
          if (r === null) status.textContent = "The test needs an account; it runs automatically when you finish setup.";
          else status.textContent = (r.ok ? "OK: " : "Failed: ") + r.message;
        } catch (e: any) { status.textContent = e.message; }
      } }, "Test storage"),
      h("button", { class: "primary", onclick: async (ev: Event) => {
        const b = ev.currentTarget as HTMLButtonElement; b.disabled = true;
        try {
          const res = await fetch("/api/v1/setup", { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ username: user.value, display_name: name.value, email: email.value || null, password: pw.value, storage: sf.value() }) });
          const j = await res.json();
          if (!res.ok) throw new Error(j.error?.message ?? "setup failed");
          useLogin(j); location.hash = "#/overview"; render();
        } catch (e: any) { status.textContent = e.message; b.disabled = false; }
      } }, "Create account and finish setup"))));
}

function renderRestore(m: ServerMeta) {
  app.replaceChildren(h("div", { class: "login card", style: "max-width:560px" }, h("h2", {}, "Restore " + m.name + " from a backup"),
    restoreSetupForm(() => { location.hash = "#/overview"; render(); }),
    h("p", {}, h("a", { href: "#", onclick: (e: Event) => { e.preventDefault(); render(); } }, "Back to set-up"))));
}

function renderLogin(m: ServerMeta) {
  const user = h("input", { autocomplete: "username", autofocus: true });
  const pw = h("input", { type: "password", autocomplete: "current-password" });
  const err = h("div", { class: "pill bad", style: "display:none;margin-top:10px" });
  const go = async () => {
    try { await login(user.value, pw.value); render(); } catch (e: any) { err.style.display = "inline-block"; err.textContent = e.message; }
  };
  pw.addEventListener("keydown", (e) => { if ((e as KeyboardEvent).key === "Enter") go(); });
  app.replaceChildren(h("div", { class: "login card" }, h("h2", {}, m.name + " admin"), field("Username", user), field("Password", pw), err,
    h("div", { class: "row", style: "margin-top:14px" }, h("button", { class: "primary", onclick: go }, "Sign in")),
    h("p", { class: "muted" }, "Admin and owner accounts only.")));
}

// ------------------------------------------------------------------ pages

const pages: Record<Tab, (el: HTMLElement, m: ServerMeta) => Promise<void>> = {
  async overview(el) {
    const o = await api("GET", "/admin/overview");
    const stat = (l: string, v: string | number) => h("div", { class: "stat" }, h("b", {}, String(v)), h("span", { class: "muted" }, l));
    el.append(h("div", { class: "card" }, h("h2", {}, "Overview"), h("div", { class: "grid" },
      stat("Users", o.users), stat("Disabled", o.disabled_users), stat("Online now", o.online), stat("Conversations", o.conversations),
      stat("Messages", o.messages), stat("Files", o.attachments), stat("File storage used", fmtBytes(o.attachment_bytes)), stat("Active sessions", o.sessions),
      stat("Storage backend", o.storage), stat("Server version", o.server_version))));
  },

  async users(el) {
    const users: AdminUser[] = await api("GET", "/admin/users");
    const reload = () => { el.replaceChildren(); return pages.users(el, null as any); };
    const tbody = h("tbody", {});
    for (const u of users) {
      const st = u.deleted ? h("span", { class: "pill bad" }, "deleted") : u.disabled ? h("span", { class: "pill bad" }, "disabled") : h("span", { class: "pill ok" }, u.presence === "online" ? "online" : "active");
      tbody.append(h("tr", {},
        h("td", {}, h("b", {}, u.display_name), h("div", { class: "muted mono" }, u.username)),
        h("td", {}, u.role), h("td", {}, st),
        h("td", {}, u.password_set ? "" : h("span", { class: "pill warn" }, "link not used")),
        h("td", {}, fmtDate(u.last_active_real_ms)),
        h("td", {}, String(u.session_count)),
        h("td", {}, h("label", { style: "display:flex;gap:6px;align-items:center;margin:0;color:var(--text)" },
          h("input", { type: "checkbox", style: "width:auto", checked: u.privacy_controls_enabled, disabled: u.deleted,
            onchange: async (e: Event) => { try { await api("PATCH", `/admin/users/${u.id}`, { privacy_controls_enabled: (e.target as HTMLInputElement).checked }); toast("Saved"); } catch (er) { fail(er); } } }), "on")),
        h("td", {}, u.deleted ? "" : userActions(u, reload))));
    }
    el.append(h("div", { class: "card" },
      h("div", { class: "row" }, h("h2", { style: "margin:0;flex:1" }, "Users"), h("button", { class: "primary", onclick: () => createUserDialog(reload) }, "Create user")),
      h("table", {}, h("thead", {}, h("tr", {}, ...["Name", "Role", "Status", "Setup", "Last active", "Sessions", "Privacy screen", "Actions"].map((x) => h("th", {}, x)))), tbody),
      h("p", { class: "muted" }, "Privacy screen: when on, the user can hide their last active time from the app's Settings. It is off for everyone by default.")));
  },

  async conversations(el) {
    const [convs, users]: [AdminConversation[], AdminUser[]] = await Promise.all([api("GET", "/admin/conversations"), api("GET", "/admin/users")]);
    const name = (id: string) => users.find((u) => u.id === id)?.display_name ?? id.slice(0, 6);
    const reload = () => { el.replaceChildren(); return pages.conversations(el, null as any); };
    const tbody = h("tbody", {});
    for (const c of convs) {
      tbody.append(h("tr", {},
        h("td", {}, h("b", {}, c.name ?? "(unnamed)"), h("div", { class: "muted" }, c.topic ?? "")),
        h("td", {}, c.kind === "channel" ? "Private channel" : "Group DM"),
        h("td", {}, c.members.map((m) => name(m.user_id)).join(", ")),
        h("td", {}, String(c.message_count)),
        h("td", {}, c.archived ? h("span", { class: "pill warn" }, "archived") : ""),
        h("td", { class: "row" },
          h("button", { onclick: () => membersDialog(c, users, reload) }, "Members"),
          h("button", { onclick: () => renameDialog(c, reload) }, "Rename"),
          h("button", { onclick: async () => { try { await api("PATCH", `/conversations/${c.id}`, { archived: !c.archived }); reload(); } catch (e) { fail(e); } } }, c.archived ? "Unarchive" : "Archive"),
          h("button", { class: "danger", onclick: () => confirmType(`Delete "${c.name}"`, `This deletes all ${c.message_count} messages and files in it for everyone. Type the name to confirm.`, c.name ?? "", async () => { await api("DELETE", `/admin/conversations/${c.id}`); reload(); }) }, "Delete"))));
    }
    el.append(h("div", { class: "card" },
      h("div", { class: "row" }, h("h2", { style: "margin:0;flex:1" }, "Channels and group DMs"), h("button", { class: "primary", onclick: () => createConvDialog(users, reload) }, "Create")),
      h("p", { class: "muted" }, "All channels are private: members only, hidden from everyone else. Only admins create them."),
      h("table", {}, h("thead", {}, h("tr", {}, ...["Name", "Kind", "Members", "Messages", "", "Actions"].map((x) => h("th", {}, x)))), tbody)));
  },

  async storage(el) {
    const info = await api("GET", "/admin/storage");
    const sf = storageForm(info.config as StorageConfig);
    const status = h("div", { style: "margin-top:10px" });
    const bar = h("div", { style: "width:0%" });
    const progress = h("div", { class: "progress", style: "display:none;margin-top:10px" }, bar);
    const poll = async (job: JobStatus) => {
      progress.style.display = "block";
      for (;;) {
        const j: JobStatus = await api("GET", `/admin/jobs/${job.id}`);
        bar.style.width = (j.total ? Math.round((j.done / j.total) * 100) : 100) + "%";
        status.textContent = `${j.kind.replace("storage_", "")}: ${j.state}. ${j.message}`;
        status.className = j.state === "failed" ? "pill bad" : "pill";
        if (j.errors.length) status.append(h("pre", { class: "mono", style: "white-space:pre-wrap" }, j.errors.slice(0, 10).join("\n")));
        if (j.state !== "running") return j;
        await new Promise((r) => setTimeout(r, 600));
      }
    };
    el.append(h("div", { class: "card" }, h("h2", {}, "Attachment storage"),
      h("p", {}, `Current backend: `, h("b", {}, info.config.backend === "s3" ? info.config.provider : info.config.backend), `, ${info.attachments} files, ${fmtBytes(info.bytes)}. Only configuration, accounts and the text database live on this server's disk.`),
      sf.el,
      h("div", { class: "row", style: "margin-top:14px" },
        h("button", { onclick: async () => { status.textContent = "Testing..."; try { const r = await api("POST", "/admin/storage/test", sf.value()); status.textContent = (r.ok ? "OK: " : "Failed: ") + r.message; status.className = "pill " + (r.ok ? "ok" : "bad"); } catch (e) { fail(e); } } }, "Test"),
        h("button", { class: "primary", onclick: async () => { try { await api("PUT", "/admin/storage", sf.value()); toast("Saved"); render(); } catch (e) { fail(e); } } }, "Save (no files yet)"),
        h("button", { onclick: () => confirmType("Move all files", "Every file is copied to the chosen storage, verified by size and checksum, then the old copies are deleted. Uploads pause meanwhile. Type MOVE to start.", "MOVE", async () => {
          const j = await api("POST", "/admin/storage/migrate", { target: sf.value(), delete_source: true }); const f = await poll(j); if (f.state === "done") render();
        }) }, "Move all files to this storage"),
        h("button", { onclick: async () => { try { const j = await api("POST", "/admin/storage/verify", { deep: false }); await poll(j); } catch (e) { fail(e); } } }, "Verify files"),
        h("button", { onclick: async () => { try { const j = await api("POST", "/admin/storage/verify", { deep: true }); await poll(j); } catch (e) { fail(e); } } }, "Deep verify (checksums)")),
      progress, status));
  },

  async sessions(el) {
    const list: SessionInfo[] = await api("GET", "/admin/sessions");
    const reload = () => { el.replaceChildren(); return pages.sessions(el, null as any); };
    el.append(h("div", { class: "card" }, h("h2", {}, "Signed-in devices"),
      h("table", {}, h("thead", {}, h("tr", {}, ...["User", "Device", "Version", "IP", "Last seen", ""].map((x) => h("th", {}, x)))),
        h("tbody", {}, ...list.map((s) => h("tr", {}, h("td", {}, s.username ?? ""), h("td", {}, s.device_name), h("td", {}, s.client_version), h("td", {}, s.ip), h("td", {}, fmtDate(s.last_seen_ms)),
          h("td", {}, s.current ? h("span", { class: "pill" }, "this browser") : h("button", { class: "danger", onclick: async () => { try { await api("DELETE", `/sessions/${s.id}`); reload(); } catch (e) { fail(e); } } }, "Sign out device"))))))));
  },

  async chats(el) {
    await chatsPage(el);
  },

  async groups(el) {
    const reload = () => { el.replaceChildren(); pages.groups(el, null as any).catch(fail); };
    await groupsPage(el, reload, fail);
  },

  async bookmarks(el) {
    const reload = () => { el.replaceChildren(); pages.bookmarks(el, null as any).catch(fail); };
    await bookmarksPage(el, reload, fail);
  },

  async bots(el) {
    const reload = () => { el.replaceChildren(); pages.bots(el, null as any).catch(fail); };
    await botsPage(el, reload, fail);
  },

  async webhooks(el) {
    const reload = () => { el.replaceChildren(); pages.webhooks(el, null as any).catch(fail); };
    await webhooksPage(el, reload, fail);
  },

  async ai(el) {
    const reload = () => { el.replaceChildren(); pages.ai(el, null as any).catch(fail); };
    await aiPage(el, reload, fail);
  },

  async backups(el) {
    const reload = () => { el.replaceChildren(); pages.backups(el, null as any).catch(fail); };
    await backupsPage(el, reload, fail);
  },
};

// ------------------------------------------------------------------ dialogs and actions

function userActions(u: AdminUser, reload: () => void): HTMLElement {
  const run = (f: () => Promise<any>, ok?: string) => async () => { try { await f(); if (ok) toast(ok); reload(); } catch (e) { fail(e); } };
  const row = h("div", { class: "row" });
  row.append(
    u.disabled
      ? h("button", { onclick: run(() => api("PATCH", `/admin/users/${u.id}`, { disabled: false }), "Enabled") }, "Enable")
      : h("button", { onclick: run(() => api("PATCH", `/admin/users/${u.id}`, { disabled: true }), "Disabled and signed out of all devices") }, "Disable"),
    h("button", { onclick: () => resetDialog(u) }, "Reset password"),
    h("button", { onclick: async () => { try { const l = await api("POST", `/admin/users/${u.id}/login-link`, {}); showLink("One-time login link for " + u.display_name, l.link, l.expires_ms); } catch (e) { fail(e); } } }, "Login link"),
    h("button", { onclick: run(async () => { const b = await api("POST", `/admin/users/${u.id}/backup`, {}); toast("Backup written: " + b.key); }) }, "Back up"),
    h("button", { class: "danger", onclick: () => confirmType("Clear history", `Backs up ${u.display_name}'s account and chats first, then removes their copy of every chat, moves files nobody else can see into the backup folder, and signs them out everywhere. Other people's copies stay. Type the username to confirm.`, u.username, async () => {
      const r = await api("POST", `/admin/users/${u.id}/clear-history`, {}); toast(`Cleared: ${r.messages_hidden} messages hidden, ${r.attachments_moved} files moved. Backup: ${r.backup_key}`); reload(); }) }, "Clear history"),
    h("button", { class: "danger", onclick: () => confirmType("Delete user", `Backs up first, then deletes the account of ${u.display_name}. Other people's copies of chats stay. Type the username to confirm.`, u.username, async () => {
      await api("DELETE", `/admin/users/${u.id}`); reload(); }) }, "Delete"));
  return row;
}

function showLink(title: string, link: string, expires: number) {
  const qr = qrSvg(link, 220);
  dialog(title, h("div", {}, h("p", {}, "Send this to the user. They paste it on the app's login page, or scan the QR code with the app (Sign in, Scan QR). It works once and expires on " + fmtDate(expires) + "."),
    h("div", { class: "linkbox mono", id: "login-link" }, link),
    h("div", { class: "qrbox", id: "login-qr" }, qr ?? h("p", { class: "muted" }, "This link is too long for a QR code; copy it instead."))), [{ label: "Copy link", primary: true, action: async () => { await copy(link); } }]);
}

function createUserDialog(reload: () => void) {
  const user = h("input", { autocomplete: "off" }), name = h("input", {}), pw = h("input", { type: "password", placeholder: "leave empty to only create a login link", autocomplete: "new-password" });
  const role = h("select", {}, h("option", { value: "member" }, "Member"), h("option", { value: "admin" }, "Admin"));
  dialog("Create user", h("div", {}, field("Username", user), field("Display name", name), field("Password (optional)", pw), field("Role", role)), [{
    label: "Create", primary: true, action: async (close) => {
      const r = await api("POST", "/admin/users", { username: user.value.trim(), display_name: name.value.trim() || user.value.trim(), password: pw.value || null, role: role.value, create_login_link: true });
      close(); reload();
      if (r.login_link) showLink("User created: one-time login link", r.login_link.link, r.login_link.expires_ms);
    },
  }]);
}

function resetDialog(u: AdminUser) {
  const pw = h("input", { type: "password", placeholder: "leave empty to generate a one-time login link", autocomplete: "new-password" });
  dialog("Reset password for " + u.display_name, h("div", {}, h("p", { class: "muted" }, "All of their devices are signed out immediately and their local data is wiped."), field("New password", pw)), [{
    label: "Reset", danger: true, action: async (close) => {
      const r = await api("POST", `/admin/users/${u.id}/reset-password`, { new_password: pw.value || null });
      close();
      if (r.login_link) showLink("New one-time login link", r.login_link.link, Date.now() + 7 * 86400000); else toast("Password reset");
    },
  }]);
}

function confirmType(title: string, text: string, expect: string, action: () => Promise<void>) {
  const i = h("input", { autocomplete: "off" });
  dialog(title, h("div", {}, h("p", {}, text), i), [{ label: title, danger: true, action: async (close) => {
    if (i.value.trim() !== expect) throw new Error("The text does not match.");
    close(); try { await action(); } catch (e) { fail(e); }
  } }]);
}

function createConvDialog(users: AdminUser[], reload: () => void) {
  const name = h("input", {}), topic = h("input", {});
  const kind = h("select", {}, h("option", { value: "channel" }, "Private channel"), h("option", { value: "group_dm" }, "Group DM"));
  const boxes = users.filter((u) => !u.disabled && !u.deleted).map((u) => ({ u, c: h("input", { type: "checkbox", style: "width:auto" }) }));
  dialog("Create conversation", h("div", {}, field("Kind", kind), field("Name", name), field("Topic", topic), h("h3", {}, "Members"),
    h("div", { style: "max-height:220px;overflow:auto" }, ...boxes.map(({ u, c }) => h("label", { style: "display:flex;gap:8px;align-items:center;color:var(--text)" }, c, u.display_name + " (" + u.username + ")")))), [{
    label: "Create", primary: true, action: async (close) => {
      await api("POST", "/conversations", { kind: kind.value, name: name.value, topic: topic.value || null, members: boxes.filter((b) => b.c.checked).map((b) => b.u.id) });
      close(); reload();
    },
  }]);
}

function membersDialog(c: AdminConversation, users: AdminUser[], reload: () => void) {
  const inConv = new Set(c.members.map((m) => m.user_id));
  const list = h("div", {});
  const sel = h("select", {}, ...users.filter((u) => !inConv.has(u.id) && !u.disabled && !u.deleted).map((u) => h("option", { value: u.id }, u.display_name + " (" + u.username + ")")));
  for (const m of c.members) {
    const u = users.find((x) => x.id === m.user_id);
    list.append(h("div", { class: "row", style: "justify-content:space-between;padding:4px 0" }, h("span", {}, u ? u.display_name + " (" + u.username + ")" : m.user_id),
      h("button", { class: "danger", onclick: async () => { try { await api("DELETE", `/admin/conversations/${c.id}/members/${m.user_id}`); document.querySelector("dialog")?.remove(); reload(); } catch (e) { fail(e); } } }, "Remove")));
  }
  dialog("Members of " + c.name, h("div", {}, list, h("h3", {}, "Add a member"), sel), [{
    label: "Add", primary: true, action: async (close) => { if (!sel.value) return; await api("POST", `/admin/conversations/${c.id}/members`, { user_ids: [sel.value] }); close(); reload(); },
  }]);
}

function renameDialog(c: AdminConversation, reload: () => void) {
  const n = h("input", { value: c.name ?? "" }), t = h("input", { value: c.topic ?? "" });
  dialog("Rename", h("div", {}, field("Name", n), field("Topic", t)), [{ label: "Save", primary: true, action: async (close) => { await api("PATCH", `/conversations/${c.id}`, { name: n.value, topic: t.value }); close(); reload(); } }]);
}

render();

// On phones tables are shown as cards; each cell is labelled with its column heading (CSS prints the label).
function labelCells() {
  for (const t of app.querySelectorAll("table")) {
    const heads = [...t.querySelectorAll("thead th")].map((x) => x.textContent ?? "");
    for (const tr of t.querySelectorAll("tbody tr")) [...tr.children].forEach((td, i) => { if (!td.hasAttribute("data-label")) td.setAttribute("data-label", heads[i] ?? ""); });
  }
}
new MutationObserver(labelCells).observe(app, { childList: true, subtree: true });
