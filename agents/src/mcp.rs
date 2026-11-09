//! A minimal MCP server (JSON-RPC 2.0, one message per line on stdio) that
//! exposes [`crate::tools`]: any MCP-speaking agent can post, hire, deliver,
//! check, buy notes and settle with the same powers and limits as ours.

use crate::chain::Acc;
use crate::tools::{list, Tools};
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

pub const PROTOCOL: &str = "2024-11-05";

/// Answer one JSON-RPC message (None for notifications).
pub async fn handle(tools: &Tools<'_>, who: &Acc, msg: &Value) -> Option<Value> {
    let id = msg.get("id")?.clone();
    let method = msg["method"].as_str().unwrap_or_default();
    let ok = |result: Value| Some(json!({ "jsonrpc": "2.0", "id": id, "result": result }));
    match method {
        "initialize" => ok(json!({
            "protocolVersion": PROTOCOL,
            "capabilities": { "tools": {} },
            "serverInfo": { "name": "hire-tree", "version": env!("CARGO_PKG_VERSION") },
            "instructions": format!("Hire Tree on NEAR. You act as {}. Amounts are fUSD with 6 decimals, as strings.", who.id),
        })),
        "ping" => ok(json!({})),
        "tools/list" => ok(json!({ "tools": list().into_iter()
            .map(|(name, description, schema)| json!({ "name": name, "description": description, "inputSchema": schema }))
            .collect::<Vec<_>>() })),
        "tools/call" => {
            let name = msg["params"]["name"].as_str().unwrap_or_default();
            let args = msg["params"].get("arguments").cloned().unwrap_or(json!({}));
            let (text, err) = match tools.call(who, name, &args).await {
                Ok(v) => (v.to_string(), false),
                Err(e) => (e, true),
            };
            ok(json!({ "content": [{ "type": "text", "text": text }], "isError": err }))
        }
        _ => Some(json!({ "jsonrpc": "2.0", "id": id,
                          "error": { "code": -32601, "message": format!("unknown method {method}") } })),
    }
}

/// Serve on stdin/stdout until EOF.
pub async fn serve(tools: Tools<'_>, who: Acc) {
    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    let mut out = tokio::io::stdout();
    while let Ok(Some(line)) = lines.next_line().await {
        let reply = match serde_json::from_str::<Value>(&line) {
            Ok(m) => {
                use futures::FutureExt;
                // A panic in one call is an error reply, not the end of the server.
                std::panic::AssertUnwindSafe(handle(&tools, &who, &m))
                    .catch_unwind()
                    .await
                    .unwrap_or_else(|_| {
                        Some(json!({ "jsonrpc": "2.0", "id": m.get("id").cloned(),
                                     "error": { "code": -32603, "message": "internal error" } }))
                    })
            }
            Err(e) => Some(
                json!({ "jsonrpc": "2.0", "id": null, "error": { "code": -32700, "message": e.to_string() } }),
            ),
        };
        if let Some(r) = reply {
            let _ = out.write_all(format!("{r}\n").as_bytes()).await;
            let _ = out.flush().await;
        }
    }
}
