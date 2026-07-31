// Trellis Web Clipper — clip the page or selection into a Trellis basket via the
// key-gated LAN API. Settings persist in extension storage.

const $ = (id) => document.getElementById(id);
const FIELDS = ["server", "key", "node"];

async function loadSettings() {
  const s = await chrome.storage.local.get(FIELDS);
  for (const f of FIELDS) $(f).value = s[f] || "";
}

function saveSettings() {
  const s = {};
  for (const f of FIELDS) s[f] = $(f).value.trim();
  chrome.storage.local.set(s);
}

function setStatus(msg, isErr) {
  const el = $("status");
  el.textContent = msg;
  el.className = isErr ? "err" : "";
}

// Pull title/url/selection out of the active tab.
async function pageInfo() {
  const [tab] = await chrome.tabs.query({ active: true, currentWindow: true });
  const [{ result }] = await chrome.scripting.executeScript({
    target: { tabId: tab.id },
    func: () => ({
      title: document.title,
      url: location.href,
      selection: String(window.getSelection() || ""),
    }),
  });
  return result;
}

async function clip(withSelection) {
  saveSettings();
  const server = $("server").value.trim().replace(/\/+$/, "");
  const key = $("key").value.trim();
  const node = $("node").value.trim();
  if (!server || !key || !node) {
    setStatus("Fill in server, key and target basket first.", true);
    return;
  }
  try {
    const info = await pageInfo();
    const sel = (info.selection || "").trim();
    if (withSelection && !sel) {
      setStatus("No text selected on the page.", true);
      return;
    }
    // A tidy Markdown text card: source link, then the quoted selection.
    let body = `[${info.title}](${info.url})`;
    if (withSelection && sel) {
      const quoted = sel.split("\n").map((l) => "> " + l).join("\n");
      body = `${quoted}\n\n— ${body}`;
    }
    const res = await fetch(`${server}/api/nodes/${encodeURIComponent(node)}/cards`, {
      method: "POST",
      headers: { "Content-Type": "application/json", "X-Api-Key": key },
      body: JSON.stringify({ kind: "text", title: info.title.slice(0, 80), body, fit: true }),
    });
    if (res.ok) {
      setStatus("Clipped to Trellis ✓");
    } else {
      const t = await res.text();
      setStatus(`Trellis error ${res.status}: ${t.slice(0, 120)}`, true);
    }
  } catch (e) {
    setStatus(`Couldn't reach Trellis: ${e.message}`, true);
  }
}

loadSettings();
FIELDS.forEach((f) => $(f).addEventListener("change", saveSettings));
$("clip-sel").addEventListener("click", () => clip(true));
$("clip-page").addEventListener("click", () => clip(false));
