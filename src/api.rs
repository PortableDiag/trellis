//! Local HTTP API so external agents can read and edit the document.
//!
//! A background thread runs a tiny blocking HTTP server bound to `127.0.0.1`.
//! Each request is authenticated against the key set in Settings, then handed to
//! the UI thread over a channel; the UI thread applies it to the live `Document`
//! and sends a response back. This keeps all document access single-threaded.
//!
//! Auth: send the key as `X-API-Key: <key>` or `Authorization: Bearer <key>`.
//! An empty key (the default) disables the API. `GET /api/health` is unauthenticated.

use crate::model::{Card, CardKind, ChecklistItem, Document, NodeId};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::mpsc::{Sender, SyncSender};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tiny_http::Method;

/// A request routed to the UI thread, plus the channel to answer on.
pub struct ApiCommand {
    pub req: ApiRequest,
    pub resp: SyncSender<ApiResponse>,
}

/// A parsed, validated API request. Document access happens on the UI thread.
pub enum ApiRequest {
    Health,
    Tree,
    ListNodes,
    GetNode(NodeId),
    ListCards(NodeId),
    CreateNode { parent: Option<NodeId>, title: String },
    UpdateNode { id: NodeId, title: Option<String>, color: Option<[u8; 3]> },
    DeleteNode(NodeId),
    AddCard { node: NodeId, input: AddCardInput },
    UpdateCard { node: NodeId, card: u64, title: Option<String>, body: Option<String> },
    DeleteCard { node: NodeId, card: u64 },
    Search(String),
}

pub struct ApiResponse {
    pub status: u16,
    pub body: String,
}

impl ApiResponse {
    fn json(status: u16, v: Value) -> Self {
        Self { status, body: serde_json::to_string_pretty(&v).unwrap_or_else(|_| "{}".into()) }
    }
    fn ok(v: Value) -> Self {
        Self::json(200, v)
    }
    fn created(v: Value) -> Self {
        Self::json(201, v)
    }
    fn err(status: u16, msg: &str) -> Self {
        Self::json(status, json!({ "error": msg }))
    }
}

// --- request DTOs -----------------------------------------------------------

#[derive(Deserialize)]
struct CreateNodeInput {
    #[serde(default)]
    parent: Option<NodeId>,
    title: String,
}

#[derive(Deserialize)]
struct UpdateNodeInput {
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    color: Option<[u8; 3]>,
}

#[derive(Deserialize)]
pub struct AddCardInput {
    #[serde(default = "default_kind")]
    kind: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    body: String,
    #[serde(default)]
    lang: Option<String>,
    #[serde(default)]
    items: Option<Vec<ChecklistItemInput>>,
    #[serde(default)]
    pos: Option<[f32; 2]>,
}

fn default_kind() -> String {
    "text".to_string()
}

#[derive(Clone, Deserialize)]
struct ChecklistItemInput {
    #[serde(default)]
    done: bool,
    #[serde(default)]
    text: String,
}

#[derive(Deserialize)]
struct UpdateCardInput {
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    body: Option<String>,
}

// --- server thread ----------------------------------------------------------

/// Bind the server (reporting bind errors synchronously) and spawn its accept
/// loop. Returns `Err` if the port can't be bound.
pub fn serve(
    port: u16,
    ctx: egui::Context,
    tx: Sender<ApiCommand>,
    key: Arc<Mutex<String>>,
) -> Result<(), String> {
    let server = tiny_http::Server::http(("127.0.0.1", port)).map_err(|e| e.to_string())?;
    std::thread::Builder::new()
        .name("trellis-api".into())
        .spawn(move || {
            for mut request in server.incoming_requests() {
                let resp = handle(&mut request, &ctx, &tx, &key);
                let header =
                    tiny_http::Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..])
                        .unwrap();
                let http = tiny_http::Response::from_string(resp.body)
                    .with_status_code(resp.status)
                    .with_header(header);
                let _ = request.respond(http);
            }
        })
        .map_err(|e| e.to_string())?;
    Ok(())
}

fn handle(
    request: &mut tiny_http::Request,
    ctx: &egui::Context,
    tx: &Sender<ApiCommand>,
    key: &Arc<Mutex<String>>,
) -> ApiResponse {
    let method = request.method().clone();
    let raw_url = request.url().to_string();
    let (path, query) = match raw_url.split_once('?') {
        Some((p, q)) => (p.to_string(), q.to_string()),
        None => (raw_url, String::new()),
    };

    // Everything but health requires the configured key.
    let is_health = method == Method::Get && path == "/api/health";
    if !is_health {
        let configured = key.lock().map(|k| k.clone()).unwrap_or_default();
        if configured.is_empty() {
            return ApiResponse::err(403, "API disabled: set a key in Settings");
        }
        if request_key(request).as_deref() != Some(configured.as_str()) {
            return ApiResponse::err(401, "missing or invalid API key");
        }
    }

    let mut body = String::new();
    let _ = request.as_reader().read_to_string(&mut body);

    let req = match route(&method, &path, &query, &body) {
        Ok(r) => r,
        Err((code, msg)) => return ApiResponse::err(code, &msg),
    };

    let (rtx, rrx) = std::sync::mpsc::sync_channel::<ApiResponse>(1);
    if tx.send(ApiCommand { req, resp: rtx }).is_err() {
        return ApiResponse::err(503, "app not accepting requests");
    }
    ctx.request_repaint(); // wake the UI thread to process the command
    match rrx.recv_timeout(Duration::from_secs(5)) {
        Ok(r) => r,
        Err(_) => ApiResponse::err(504, "timed out waiting for the app"),
    }
}

fn request_key(request: &tiny_http::Request) -> Option<String> {
    for h in request.headers() {
        let field = h.field.as_str().as_str().to_ascii_lowercase();
        if field == "x-api-key" {
            return Some(h.value.as_str().to_string());
        }
        if field == "authorization" {
            if let Some(tok) = h.value.as_str().strip_prefix("Bearer ") {
                return Some(tok.to_string());
            }
        }
    }
    None
}

fn route(method: &Method, path: &str, query: &str, body: &str) -> Result<ApiRequest, (u16, String)> {
    let seg: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    match (method, seg.as_slice()) {
        (Method::Get, ["api", "health"]) => Ok(ApiRequest::Health),
        (Method::Get, ["api", "tree"]) => Ok(ApiRequest::Tree),
        (Method::Get, ["api", "nodes"]) => Ok(ApiRequest::ListNodes),
        (Method::Post, ["api", "nodes"]) => {
            let i: CreateNodeInput = parse(body)?;
            Ok(ApiRequest::CreateNode { parent: i.parent, title: i.title })
        }
        (Method::Get, ["api", "nodes", id]) => Ok(ApiRequest::GetNode(pid(id)?)),
        (Method::Patch, ["api", "nodes", id]) => {
            let i: UpdateNodeInput = parse(body)?;
            Ok(ApiRequest::UpdateNode { id: pid(id)?, title: i.title, color: i.color })
        }
        (Method::Delete, ["api", "nodes", id]) => Ok(ApiRequest::DeleteNode(pid(id)?)),
        (Method::Get, ["api", "nodes", id, "cards"]) => Ok(ApiRequest::ListCards(pid(id)?)),
        (Method::Post, ["api", "nodes", id, "cards"]) => {
            let input: AddCardInput = parse(body)?;
            Ok(ApiRequest::AddCard { node: pid(id)?, input })
        }
        (Method::Patch, ["api", "nodes", nid, "cards", cid]) => {
            let i: UpdateCardInput = parse(body)?;
            Ok(ApiRequest::UpdateCard { node: pid(nid)?, card: pid(cid)?, title: i.title, body: i.body })
        }
        (Method::Delete, ["api", "nodes", nid, "cards", cid]) => {
            Ok(ApiRequest::DeleteCard { node: pid(nid)?, card: pid(cid)? })
        }
        (Method::Get, ["api", "search"]) => {
            Ok(ApiRequest::Search(query_get(query, "q").unwrap_or_default()))
        }
        _ => Err((404, format!("no route for {:?} {}", method, path))),
    }
}

fn parse<T: for<'de> Deserialize<'de>>(body: &str) -> Result<T, (u16, String)> {
    serde_json::from_str(body).map_err(|e| (400, format!("invalid JSON body: {e}")))
}

fn pid(s: &str) -> Result<u64, (u16, String)> {
    s.parse::<u64>().map_err(|_| (400, format!("bad id: {s}")))
}

fn query_get(query: &str, key: &str) -> Option<String> {
    query.split('&').find_map(|pair| {
        pair.split_once('=')
            .filter(|(k, _)| *k == key)
            .map(|(_, v)| percent_decode(v))
    })
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => match u8::from_str_radix(&s[i + 1..i + 3], 16) {
                Ok(b) => {
                    out.push(b);
                    i += 3;
                }
                Err(_) => {
                    out.push(b'%');
                    i += 1;
                }
            },
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

// --- request processing (UI thread) -----------------------------------------

/// Apply a request to the document. Returns `(document_changed, response)`.
pub fn process(doc: &mut Document, req: ApiRequest) -> (bool, ApiResponse) {
    match req {
        ApiRequest::Health => (false, ApiResponse::ok(json!({ "status": "ok", "app": "trellis" }))),
        ApiRequest::Tree => (false, ApiResponse::ok(json!({ "roots": tree_nodes(doc, &doc.roots) }))),
        ApiRequest::ListNodes => {
            let list: Vec<Value> = doc
                .nodes
                .values()
                .map(|n| {
                    json!({
                        "id": n.id,
                        "title": n.title,
                        "parent": n.parent,
                        "children": n.children,
                        "cards": n.cards.len(),
                    })
                })
                .collect();
            (false, ApiResponse::ok(json!({ "nodes": list })))
        }
        ApiRequest::GetNode(id) => match doc.nodes.get(&id) {
            Some(n) => (false, ApiResponse::ok(node_json(n))),
            None => (false, ApiResponse::err(404, "node not found")),
        },
        ApiRequest::ListCards(id) => match doc.nodes.get(&id) {
            Some(n) => {
                let cards: Vec<Value> = n.cards.iter().map(card_json).collect();
                (false, ApiResponse::ok(json!({ "cards": cards })))
            }
            None => (false, ApiResponse::err(404, "node not found")),
        },
        ApiRequest::CreateNode { parent, title } => {
            if let Some(p) = parent {
                if !doc.nodes.contains_key(&p) {
                    return (false, ApiResponse::err(400, "parent node not found"));
                }
            }
            let id = doc.add_node(parent, title);
            (true, ApiResponse::created(json!({ "id": id })))
        }
        ApiRequest::UpdateNode { id, title, color } => match doc.nodes.get_mut(&id) {
            Some(n) => {
                if let Some(t) = title {
                    n.title = t;
                }
                if let Some(c) = color {
                    n.color = Some(c);
                }
                (true, ApiResponse::ok(json!({ "id": id })))
            }
            None => (false, ApiResponse::err(404, "node not found")),
        },
        ApiRequest::DeleteNode(id) => {
            if !doc.nodes.contains_key(&id) {
                return (false, ApiResponse::err(404, "node not found"));
            }
            doc.remove_node(id);
            (true, ApiResponse::ok(json!({ "deleted": id })))
        }
        ApiRequest::AddCard { node, input } => {
            if !doc.nodes.contains_key(&node) {
                return (false, ApiResponse::err(404, "node not found"));
            }
            let kind = match input.kind.as_str() {
                "code" => CardKind::Code { lang: input.lang.clone().unwrap_or_else(|| "text".into()) },
                "checklist" => CardKind::Checklist {
                    items: input
                        .items
                        .clone()
                        .unwrap_or_default()
                        .into_iter()
                        .map(|i| ChecklistItem { done: i.done, text: i.text })
                        .collect(),
                },
                "image" => CardKind::Image { data: Vec::new(), name: input.title.clone() },
                _ => CardKind::Text,
            };
            let pos = input
                .pos
                .map(|[x, y]| egui::pos2(x, y))
                .unwrap_or_else(|| egui::pos2(40.0, 40.0));
            match doc.add_card(node, pos, kind) {
                Some(cid) => {
                    if let Some(c) = doc.card_mut(node, cid) {
                        c.title = input.title;
                        c.body = input.body;
                        c.editing = false;
                    }
                    (true, ApiResponse::created(json!({ "id": cid })))
                }
                None => (false, ApiResponse::err(404, "node not found")),
            }
        }
        ApiRequest::UpdateCard { node, card, title, body } => match doc.card_mut(node, card) {
            Some(c) => {
                if let Some(t) = title {
                    c.title = t;
                }
                if let Some(b) = body {
                    c.body = b;
                }
                (true, ApiResponse::ok(json!({ "id": card })))
            }
            None => (false, ApiResponse::err(404, "card not found")),
        },
        ApiRequest::DeleteCard { node, card } => {
            let existed = doc
                .nodes
                .get(&node)
                .map(|n| n.cards.iter().any(|c| c.id == card))
                .unwrap_or(false);
            if !existed {
                return (false, ApiResponse::err(404, "card not found"));
            }
            doc.remove_card(node, card);
            (true, ApiResponse::ok(json!({ "deleted": card })))
        }
        ApiRequest::Search(q) => {
            let hits: Vec<Value> = doc
                .search(&q)
                .into_iter()
                .map(|h| json!({ "node": h.node, "node_title": h.node_title, "snippet": h.snippet }))
                .collect();
            (false, ApiResponse::ok(json!({ "hits": hits })))
        }
    }
}

fn tree_nodes(doc: &Document, ids: &[NodeId]) -> Vec<Value> {
    ids.iter()
        .filter_map(|id| doc.nodes.get(id))
        .map(|n| {
            json!({
                "id": n.id,
                "title": n.title,
                "color": n.color,
                "cards": n.cards.len(),
                "children": tree_nodes(doc, &n.children),
            })
        })
        .collect()
}

fn node_json(n: &crate::model::Node) -> Value {
    json!({
        "id": n.id,
        "title": n.title,
        "parent": n.parent,
        "children": n.children,
        "color": n.color,
        "cards": n.cards.iter().map(card_json).collect::<Vec<_>>(),
    })
}

fn card_json(c: &Card) -> Value {
    let mut v = json!({ "id": c.id, "title": c.title, "kind": c.kind.label().to_lowercase() });
    match &c.kind {
        CardKind::Text => {
            v["body"] = json!(c.body);
        }
        CardKind::Code { lang } => {
            v["body"] = json!(c.body);
            v["lang"] = json!(lang);
        }
        CardKind::Checklist { items } => {
            v["items"] = json!(items
                .iter()
                .map(|i| json!({ "done": i.done, "text": i.text }))
                .collect::<Vec<_>>());
        }
        CardKind::Image { name, data } => {
            v["image_name"] = json!(name);
            v["bytes"] = json!(data.len());
        }
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body_id(resp: &ApiResponse) -> u64 {
        serde_json::from_str::<Value>(&resp.body).unwrap()["id"].as_u64().unwrap()
    }

    #[test]
    fn routes_parse() {
        assert!(matches!(route(&Method::Get, "/api/tree", "", "").unwrap(), ApiRequest::Tree));
        assert!(matches!(
            route(&Method::Get, "/api/nodes/5", "", "").unwrap(),
            ApiRequest::GetNode(5)
        ));
        assert!(matches!(
            route(&Method::Delete, "/api/nodes/5/cards/9", "", "").unwrap(),
            ApiRequest::DeleteCard { node: 5, card: 9 }
        ));
        assert!(matches!(
            route(&Method::Get, "/api/search", "q=hello%20world", "").unwrap(),
            ApiRequest::Search(q) if q == "hello world"
        ));
        assert!(route(&Method::Get, "/api/bogus", "", "").is_err());
        assert!(route(&Method::Get, "/api/nodes/notanumber", "", "").is_err());
    }

    #[test]
    fn create_read_update_delete_node() {
        let mut doc = Document::empty();
        let (dirty, resp) =
            process(&mut doc, ApiRequest::CreateNode { parent: None, title: "Test".into() });
        assert!(dirty);
        assert_eq!(resp.status, 201);
        let id = body_id(&resp);

        let (_, got) = process(&mut doc, ApiRequest::GetNode(id));
        assert_eq!(got.status, 200);
        assert!(got.body.contains("Test"));

        let (_, up) = process(
            &mut doc,
            ApiRequest::UpdateNode { id, title: Some("Renamed".into()), color: None },
        );
        assert_eq!(up.status, 200);
        assert_eq!(doc.nodes[&id].title, "Renamed");

        let (_, del) = process(&mut doc, ApiRequest::DeleteNode(id));
        assert_eq!(del.status, 200);
        assert!(!doc.nodes.contains_key(&id));
    }

    #[test]
    fn add_card_then_search_finds_it() {
        let mut doc = Document::empty();
        let nid = doc.add_node(None, "Node".into());
        let input: AddCardInput =
            serde_json::from_str(r#"{"kind":"text","title":"hi","body":"needle"}"#).unwrap();
        let (dirty, resp) = process(&mut doc, ApiRequest::AddCard { node: nid, input });
        assert!(dirty);
        assert_eq!(resp.status, 201);

        let (_, s) = process(&mut doc, ApiRequest::Search("needle".into()));
        assert_eq!(s.status, 200);
        assert!(s.body.contains("needle"));
    }

    #[test]
    fn missing_node_is_404() {
        let mut doc = Document::empty();
        assert_eq!(process(&mut doc, ApiRequest::GetNode(999)).1.status, 404);
        assert_eq!(process(&mut doc, ApiRequest::DeleteNode(999)).1.status, 404);
    }

    #[test]
    fn create_node_with_missing_parent_is_400() {
        let mut doc = Document::empty();
        let (dirty, resp) =
            process(&mut doc, ApiRequest::CreateNode { parent: Some(42), title: "x".into() });
        assert!(!dirty);
        assert_eq!(resp.status, 400);
    }
}
