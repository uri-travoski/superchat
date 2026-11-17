// Admin pages for automation: bots (SV-15), webhooks (SV-16) and AI settings (SV-21).
import { api } from "./api";
import type { AdminBot, AdminConversation, AiSettings, IncomingWebhook, OutgoingWebhook, WebhooksOverview } from "./api-types";
import { copy, dialog, field, fmtDate, h, toast } from "./dom";

type Fail = (e: unknown) => void;

const EVENTS = ["message.created", "message.updated", "message.deleted", "form.submitted"];

function secretBox(label: string, value: string): HTMLElement {
  const code = h("code", { class: "mono secret", "data-secret": label }, "•".repeat(12));
  let shown = false;
  return h("div", { class: "row" },
    h("span", { class: "muted" }, label), code,
    h("button", { class: "tiny", onclick: () => { shown = !shown; code.textContent = shown ? value : "•".repeat(12); } }, "Show"),
    h("button", { class: "tiny", onclick: () => copy(value) }, "Copy"));
}

// ------------------------------------------------------------------ bots

export async function botsPage(el: HTMLElement, reload: () => void, fail: Fail): Promise<void> {
  const [bots, convs] = await Promise.all([api<AdminBot[]>("GET", "/admin/bots"), api<AdminConversation[]>("GET", "/admin/conversations")]);
  const base = location.origin;
  const rows = bots.map((b) => botCard(b, convs, base, reload, fail));
  el.append(h("div", { class: "card" },
    h("div", { class: "row" }, h("h2", { style: "margin:0;flex:1" }, "Bots"), h("button", { class: "primary", id: "new-bot", onclick: () => createBotDialog(convs, reload) }, "Create bot")),
    h("p", { class: "muted" }, "A bot is an account with a bearer token. It can only post into conversations you add it to. Apps use the Telegram-style endpoints (/bot<token>/sendMessage, getUpdates, setWebhook ...) or the native REST API; messages from bots carry an APP tag. See docs/BOTS.md."),
    bots.length === 0 ? h("p", { class: "muted" }, "No bots yet.") : null),
    ...rows);
}

function botCard(b: AdminBot, convs: AdminConversation[], base: string, reload: () => void, fail: Fail): HTMLElement {
  const run = (f: () => Promise<any>, ok?: string) => async () => { try { await f(); if (ok) toast(ok); reload(); } catch (e) { fail(e); } };
  const memberOf = new Set(b.conversations.map((c) => c.id));
  const addable = convs.filter((c) => !memberOf.has(c.id) && !c.archived);
  const sel = h("select", { "aria-label": "Add to conversation" }, ...addable.map((c) => h("option", { value: c.id }, (c.kind === "channel" ? "# " : "") + (c.name ?? "(unnamed)"))));
  const hook = b.webhook;
  return h("div", { class: "card bot", "data-bot": b.user.username },
    h("div", { class: "row" },
      h("b", {}, b.user.display_name), h("span", { class: "pill" }, "APP"), h("span", { class: "muted mono" }, b.user.username),
      b.user.disabled ? h("span", { class: "pill bad" }, "disabled") : h("span", { class: "pill ok" }, "active"),
      h("div", { class: "spacer" }),
      b.user.disabled
        ? h("button", { onclick: run(() => api("PATCH", `/admin/bots/${b.user.id}`, { disabled: false }), "Enabled") }, "Enable")
        : h("button", { onclick: run(() => api("PATCH", `/admin/bots/${b.user.id}`, { disabled: true }), "Disabled") }, "Disable"),
      h("button", { onclick: () => editBotDialog(b, reload) }, "Edit"),
      h("button", { class: "danger", onclick: () => { if (confirm(`Delete the bot "${b.user.display_name}"? Its messages stay as "Deleted user".`)) run(() => api("DELETE", `/admin/bots/${b.user.id}`), "Bot deleted")(); } }, "Delete")),
    b.description ? h("p", { class: "muted" }, b.description) : null,
    h("div", { class: "grid2" },
      h("div", {}, h("h3", {}, "Token"),
        secretBox("Bearer token", b.token),
        h("p", { class: "muted small mono" }, `${base}/bot<token>/sendMessage`),
        h("button", { class: "danger tiny", onclick: () => { if (confirm("Generate a new token? The old one stops working immediately.")) run(async () => { const n = await api<AdminBot>("POST", `/admin/bots/${b.user.id}/token`, {}); showToken(n); }, undefined)(); } }, "Regenerate token")),
      h("div", {}, h("h3", {}, "Webhook"),
        hook
          ? h("div", {}, h("div", { class: "mono small" }, hook.url), h("div", { class: "muted small" }, `${hook.pending_updates} waiting${hook.has_secret ? ", secret token set" : ""}`),
            hook.last_error ? h("div", { class: "pill bad" }, `Last error ${fmtDate(hook.last_error_ms)}: ${hook.last_error}`) : h("div", { class: "pill ok" }, "No delivery errors"))
          : h("div", { class: "muted" }, `No webhook set. The bot polls getUpdates (${b.queued_updates} update${b.queued_updates === 1 ? "" : "s"} waiting).`))),
    h("h3", {}, "Member of"),
    h("div", { class: "row wrap" },
      ...(b.conversations.length ? b.conversations.map((c) => h("span", { class: "chip" }, (c.kind === "channel" ? "# " : "") + c.name, " ",
        c.kind === "dm" || c.kind === "self" ? null : h("button", { class: "tiny", title: "Remove from conversation", "aria-label": "Remove from " + c.name, onclick: run(() => api("DELETE", `/admin/bots/${b.user.id}/conversations/${c.id}`)) }, "×"))) : [h("span", { class: "muted" }, "No conversations yet. People can also start a direct message with it.")])),
    addable.length ? h("div", { class: "row", style: "margin-top:10px" }, sel, h("button", { onclick: run(() => api("PUT", `/admin/bots/${b.user.id}/conversations/${sel.value}`, {}), "Added") }, "Add to conversation")) : null);
}

function showToken(b: AdminBot) {
  dialog("New token for " + b.user.display_name, h("div", {}, h("p", {}, "Copy it now into your app. The old token no longer works."), h("div", { class: "linkbox mono" }, b.token)), [{ label: "Copy token", primary: true, action: async () => { await copy(b.token); } }]);
}

function createBotDialog(convs: AdminConversation[], reload: () => void) {
  const user = h("input", { autocomplete: "off", placeholder: "receipt_bot", "aria-label": "Bot username" });
  const name = h("input", { placeholder: "Receipt Logger", "aria-label": "Bot display name" });
  const desc = h("input", { placeholder: "What it does", "aria-label": "Description" });
  const boxes = convs.filter((c) => !c.archived).map((c) => ({ c, i: h("input", { type: "checkbox", style: "width:auto" }) }));
  dialog("Create bot", h("div", {}, field("Username", user), field("Display name", name), field("Description", desc), h("h3", {}, "Add to"),
    h("div", { style: "max-height:200px;overflow:auto" }, ...boxes.map(({ c, i }) => h("label", { style: "display:flex;gap:8px;align-items:center;color:var(--text)" }, i, (c.kind === "channel" ? "# " : "") + (c.name ?? "(unnamed)"))))), [{
    label: "Create", primary: true, action: async (close) => {
      const b = await api<AdminBot>("POST", "/admin/bots", { username: user.value.trim(), display_name: name.value.trim() || user.value.trim(), description: desc.value.trim(), conv_ids: boxes.filter((x) => x.i.checked).map((x) => x.c.id) });
      close(); reload(); showToken(b);
    },
  }]);
}

function editBotDialog(b: AdminBot, reload: () => void) {
  const name = h("input", { value: b.user.display_name }), desc = h("input", { value: b.description });
  dialog("Edit bot", h("div", {}, field("Display name", name), field("Description", desc)), [{
    label: "Save", primary: true, action: async (close) => { await api("PATCH", `/admin/bots/${b.user.id}`, { display_name: name.value, description: desc.value }); close(); reload(); },
  }]);
}

// ------------------------------------------------------------------ webhooks

export async function webhooksPage(el: HTMLElement, reload: () => void, fail: Fail): Promise<void> {
  const [w, convs] = await Promise.all([api<WebhooksOverview>("GET", "/admin/webhooks"), api<AdminConversation[]>("GET", "/admin/conversations")]);
  const convName = (id?: string | null) => convs.find((c) => c.id === id)?.name ?? "all conversations";
  const run = (f: () => Promise<any>, ok?: string) => async () => { try { await f(); if (ok) toast(ok); reload(); } catch (e) { fail(e); } };

  el.append(h("div", { class: "card", id: "incoming" },
    h("div", { class: "row" }, h("h2", { style: "margin:0;flex:1" }, "Incoming webhooks"), h("button", { class: "primary", id: "new-incoming", onclick: () => createIncomingDialog(convs, reload) }, "Create incoming webhook")),
    h("p", { class: "muted" }, "A secret URL that posts into one conversation: POST {\"text\": \"...\"} as JSON. Anyone with the URL can post, so treat it like a password. Messages carry an APP tag."),
    w.incoming.length === 0 ? h("p", { class: "muted" }, "None yet.") : h("table", {}, h("thead", {}, h("tr", {}, ...["Name", "Posts into", "URL", "Last used", ""].map((x) => h("th", {}, x)))),
      h("tbody", {}, ...w.incoming.map((i) => incomingRow(i, run, base()))))));

  el.append(h("div", { class: "card", id: "outgoing" },
    h("div", { class: "row" }, h("h2", { style: "margin:0;flex:1" }, "Outgoing webhooks"), h("button", { class: "primary", id: "new-outgoing", onclick: () => createOutgoingDialog(convs, reload) }, "Create outgoing webhook")),
    h("p", { class: "muted" }, "Events are POSTed to your URL as JSON with X-Atomic-Timestamp and X-Atomic-Signature (sha256= HMAC of \"<timestamp>.<body>\" with the secret). Failed deliveries are retried with growing delays and survive a restart."),
    w.outgoing.length === 0 ? h("p", { class: "muted" }, "None yet.") : h("table", {}, h("thead", {}, h("tr", {}, ...["Name", "URL", "Events", "Status", ""].map((x) => h("th", {}, x)))),
      h("tbody", {}, ...w.outgoing.map((o) => outgoingRow(o, convName(o.conv_id), run))))));
}

const base = () => location.origin;

function incomingRow(i: IncomingWebhook, run: (f: () => Promise<any>, ok?: string) => () => void, origin: string): HTMLElement {
  const url = origin + i.path;
  return h("tr", { "data-incoming": i.name },
    h("td", {}, h("b", {}, i.name), i.enabled ? null : h("span", { class: "pill bad" }, "disabled")),
    h("td", {}, "# " + i.conv_name),
    h("td", {}, secretBox("URL", url)),
    h("td", {}, i.last_used_ms ? fmtDate(i.last_used_ms) : "never"),
    h("td", { class: "row" },
      h("button", { onclick: run(() => api("PATCH", `/admin/webhooks/incoming/${i.id}`, { enabled: !i.enabled })) }, i.enabled ? "Disable" : "Enable"),
      h("button", { onclick: () => { if (confirm("Create a new secret URL? The old one stops working.")) run(() => api("PATCH", `/admin/webhooks/incoming/${i.id}`, { regenerate_secret: true }), "New URL created")(); } }, "New URL"),
      h("button", { class: "danger", onclick: () => { if (confirm(`Delete the webhook "${i.name}"?`)) run(() => api("DELETE", `/admin/webhooks/incoming/${i.id}`))(); } }, "Delete")));
}

function outgoingRow(o: OutgoingWebhook, conv: string, run: (f: () => Promise<any>, ok?: string) => () => void): HTMLElement {
  const status = o.last_status ? h("span", { class: "pill " + (o.last_status.startsWith("delivered") ? "ok" : o.last_status.startsWith("failed") ? "bad" : "warn") }, o.last_status) : h("span", { class: "muted" }, "no deliveries yet");
  return h("tr", { "data-outgoing": o.name },
    h("td", {}, h("b", {}, o.name), o.enabled ? null : h("span", { class: "pill bad" }, "disabled"), h("div", { class: "muted small" }, conv)),
    h("td", {}, h("div", { class: "mono small" }, o.url), secretBox("Secret", o.secret)),
    h("td", {}, o.events.length ? o.events.join(", ") : "all events"),
    h("td", {}, status, o.pending ? h("div", { class: "muted small" }, `${o.pending} queued`) : null, o.failed ? h("div", { class: "muted small" }, `${o.failed} gave up`) : null, o.last_delivery_ms ? h("div", { class: "muted small" }, fmtDate(o.last_delivery_ms)) : null),
    h("td", { class: "row" },
      h("button", { onclick: run(() => api("POST", `/admin/webhooks/outgoing/${o.id}/test`, {}), "Test ping queued") }, "Send test"),
      h("button", { onclick: run(() => api("PATCH", `/admin/webhooks/outgoing/${o.id}`, { enabled: !o.enabled })) }, o.enabled ? "Disable" : "Enable"),
      h("button", { onclick: () => { if (confirm("Generate a new secret? Update your receiver too.")) run(() => api("PATCH", `/admin/webhooks/outgoing/${o.id}`, { regenerate_secret: true }), "New secret created")(); } }, "New secret"),
      h("button", { class: "danger", onclick: () => { if (confirm(`Delete the webhook "${o.name}"?`)) run(() => api("DELETE", `/admin/webhooks/outgoing/${o.id}`))(); } }, "Delete")));
}

function createIncomingDialog(convs: AdminConversation[], reload: () => void) {
  const name = h("input", { placeholder: "CI builds", "aria-label": "Webhook name" });
  const conv = h("select", { "aria-label": "Conversation" }, ...convs.filter((c) => !c.archived).map((c) => h("option", { value: c.id }, (c.kind === "channel" ? "# " : "") + (c.name ?? "(unnamed)"))));
  dialog("Create incoming webhook", h("div", {}, field("Name (shown as the sender)", name), field("Post into", conv)), [{
    label: "Create", primary: true, action: async (close) => { await api("POST", "/admin/webhooks/incoming", { name: name.value.trim(), conv_id: conv.value }); close(); reload(); },
  }]);
}

function createOutgoingDialog(convs: AdminConversation[], reload: () => void) {
  const name = h("input", { placeholder: "Audit log", "aria-label": "Webhook name" });
  const url = h("input", { placeholder: "https://example.com/hook", "aria-label": "Receiver URL" });
  const secret = h("input", { placeholder: "leave empty to generate", "aria-label": "Secret" });
  const conv = h("select", { "aria-label": "Conversation" }, h("option", { value: "" }, "All conversations"), ...convs.map((c) => h("option", { value: c.id }, (c.kind === "channel" ? "# " : "") + (c.name ?? "(unnamed)"))));
  const boxes = EVENTS.map((e) => ({ e, i: h("input", { type: "checkbox", style: "width:auto", checked: e === "message.created" }) }));
  dialog("Create outgoing webhook", h("div", {}, field("Name", name), field("Receiver URL", url), field("Secret", secret), field("Conversation", conv), h("h3", {}, "Events"),
    ...boxes.map(({ e, i }) => h("label", { style: "display:flex;gap:8px;align-items:center;color:var(--text)" }, i, e))), [{
    label: "Create", primary: true, action: async (close) => {
      await api("POST", "/admin/webhooks/outgoing", { name: name.value.trim(), url: url.value.trim(), secret: secret.value || null, conv_id: conv.value || null, events: boxes.filter((b) => b.i.checked).map((b) => b.e) });
      close(); reload();
    },
  }]);
}

// ------------------------------------------------------------------ AI

export async function aiPage(el: HTMLElement, reload: () => void, fail: Fail): Promise<void> {
  const s = await api<AiSettings>("GET", "/admin/ai");
  const enabled = h("input", { type: "checkbox", checked: s.enabled, style: "width:auto", id: "ai-enabled", "aria-label": "Turn AI features on" });
  const provider = h("select", { "aria-label": "Provider", id: "ai-provider" }, h("option", { value: "anthropic" }, "Anthropic (Claude)"), h("option", { value: "openai" }, "OpenAI-compatible"));
  provider.value = s.provider;
  const model = h("input", { value: s.model, "aria-label": "Model", id: "ai-model" });
  const base = h("input", { value: s.base_url, placeholder: "default for the provider", "aria-label": "Base URL", id: "ai-base" });
  const key = h("input", { type: "password", autocomplete: "new-password", placeholder: s.has_key ? `set (ends in ${s.key_hint}); leave empty to keep` : "paste the API key", "aria-label": "API key", id: "ai-key" });
  const out = h("div", { class: "muted", id: "ai-status" });
  const save = async () => {
    const body: any = { enabled: enabled.checked, provider: provider.value, model: model.value, base_url: base.value };
    if (key.value) body.api_key = key.value;
    await api("PUT", "/admin/ai", body);
  };
  provider.addEventListener("change", () => { model.value = provider.value === "anthropic" ? "claude-sonnet-5-5" : "gpt-4o-mini"; });
  el.append(h("div", { class: "card" }, h("h2", {}, "AI features"),
    h("div", { class: "notice", role: "note", id: "ai-notice" }, h("b", {}, "Privacy: "), s.notice),
    h("p", { class: "muted" }, "Off by default. When on, people get natural-language search (the question and the names of people and channels they can see are sent to the provider to build a search) and channel recaps (the text of the recapped messages is sent). The key is stored on this server and never shown again."),
    h("label", { class: "radio" }, enabled, " Turn AI features on"),
    h("div", { class: "grid" }, field("Provider", provider), field("Model", model), field("Base URL (optional)", base), field("API key", key)),
    h("div", { class: "row", style: "margin-top:14px" },
      h("button", { class: "primary", id: "ai-save", onclick: async () => { try { await save(); toast("AI settings saved"); reload(); } catch (e) { fail(e); } } }, "Save"),
      h("button", { id: "ai-test", onclick: async () => { out.textContent = "Testing..."; try { await save(); const r = await api("POST", "/admin/ai/test", {}); out.textContent = (r.ok ? "OK. " : "Failed. ") + r.message; out.className = "pill " + (r.ok ? "ok" : "bad"); } catch (e) { fail(e); out.textContent = ""; } } }, "Save and test")),
    out));
}
