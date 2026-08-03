# Trellis Web Clipper

A tiny Chrome/Edge (Manifest V3) extension that clips the current page — or the
text you've selected — into a Trellis basket over the [LAN API](../API.md).

## Install (unpacked)

1. In Trellis, open **Tools → Settings**, enable **LAN access**, and copy your
   **API key**. Note the server address shown (e.g. `http://192.168.1.20:7373`).
   If you run an instance per document, the **port picks the document** clips land
   in — use the port of the instance holding the notes you're clipping into, and
   its API key (each instance has its own).
2. Right-click the basket you want clips to land in → **Copy** → **Node id**.
3. In Chrome/Edge go to `chrome://extensions`, turn on **Developer mode**, click
   **Load unpacked**, and select this `web-clipper/` folder.
4. Open the extension popup → **Settings** and fill in the server, API key, and
   target node id. They're remembered.

## Use

- **Clip selection** — sends the highlighted text (as a Markdown quote) plus a
  link back to the page, as a new text card.
- **Clip page** — sends just the page title linked to its URL.

The API is key-gated and, with LAN access on, reachable from your other devices;
only enable it on trusted networks. The extension talks to the API cross-origin,
which the API now allows via permissive CORS (the key is still required).
