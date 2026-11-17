// Tiny DOM helpers. Text is always set via textContent, never innerHTML.
type Child = Node | string | null | undefined | false;

export function h<K extends keyof HTMLElementTagNameMap>(tag: K, attrs: Record<string, any> = {}, ...children: Child[]): HTMLElementTagNameMap[K] {
  const el = document.createElement(tag);
  for (const [k, v] of Object.entries(attrs)) {
    if (v === undefined || v === null || v === false) continue;
    if (k === "class") el.className = v;
    else if (k.startsWith("on") && typeof v === "function") el.addEventListener(k.slice(2), v);
    else if (k === "value") (el as any).value = v;
    else if (k === "checked") (el as any).checked = !!v;
    else el.setAttribute(k, v === true ? "" : String(v));
  }
  for (const c of children.flat()) {
    if (c === null || c === undefined || c === false) continue;
    el.append(typeof c === "string" ? document.createTextNode(c) : c);
  }
  return el;
}

export function toast(msg: string, err = false) {
  const t = h("div", { class: "toast" + (err ? " err" : "") }, msg);
  document.body.append(t);
  setTimeout(() => t.remove(), err ? 6000 : 3000);
}

export function dialog(title: string, body: Node, buttons: { label: string; primary?: boolean; danger?: boolean; action: (close: () => void) => void | Promise<void> }[]): HTMLDialogElement {
  const d = h("dialog", {});
  const close = () => { d.close(); d.remove(); };
  d.append(h("h2", {}, title), body, h("div", { class: "row", style: "margin-top:16px;justify-content:flex-end" },
    h("button", { onclick: close }, "Cancel"),
    ...buttons.map((b) => h("button", { class: b.primary ? "primary" : b.danger ? "danger" : "", onclick: async (ev: Event) => {
      const btn = ev.currentTarget as HTMLButtonElement; btn.disabled = true;
      try { await b.action(close); } catch (e: any) { toast(e.message ?? String(e), true); } finally { btn.disabled = false; }
    } }, b.label))));
  document.body.append(d);
  d.showModal();
  return d;
}

export function field(label: string, input: HTMLElement): HTMLElement {
  return h("div", {}, h("label", {}, label), input);
}

export function fmtDate(ms?: number | null): string {
  if (!ms) return "never";
  const d = new Date(ms);
  const p = (n: number) => String(n).padStart(2, "0");
  let hr = d.getHours(); const ap = hr >= 12 ? "pm" : "am"; hr = hr % 12 || 12;
  return `${p(d.getDate())}/${p(d.getMonth() + 1)}/${d.getFullYear()} ${hr}:${p(d.getMinutes())} ${ap}`;
}

export function fmtBytes(n: number): string {
  if (n < 1024) return n + " B";
  const u = ["KB", "MB", "GB", "TB"]; let i = -1;
  do { n /= 1024; i++; } while (n >= 1024 && i < u.length - 1);
  return n.toFixed(n < 10 ? 1 : 0) + " " + u[i];
}

export async function copy(text: string) {
  try { await navigator.clipboard.writeText(text); toast("Copied"); } catch { toast("Select the text and copy it manually", true); }
}
