// notakase-mcp — a Model Context Protocol server over stdio, exposing the
// notakase note vault as tools an LLM can call. Pairs with todokase-mcp: with
// both registered, Claude can read a task and expand it into a note.
//
// Transport: newline-delimited JSON-RPC 2.0 on stdin/stdout. stdout carries
// the protocol, so all logging goes to stderr.

mod session;
mod tools;

use anyhow::Result;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

const PROTOCOL_VERSION: &str = "2024-11-05";

struct RpcError {
    code: i64,
    message: String,
}

fn rpc_err(code: i64, message: impl Into<String>) -> RpcError {
    RpcError { code, message: message.into() }
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let mut reader = BufReader::new(tokio::io::stdin()).lines();
    let mut stdout = tokio::io::stdout();

    while let Some(line) = reader.next_line().await? {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let msg: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(e) => {
                write_error(&mut stdout, Value::Null, -32700, &format!("parse error: {e}")).await?;
                continue;
            }
        };

        let id = msg.get("id").cloned();
        let method = msg.get("method").and_then(Value::as_str).unwrap_or("").to_string();
        let params = msg.get("params").cloned().unwrap_or(Value::Null);

        let Some(id) = id else {
            tracing::debug!("notification: {method}");
            continue;
        };

        match dispatch(&method, params).await {
            Ok(result) => write_result(&mut stdout, id, result).await?,
            Err(e) => write_error(&mut stdout, id, e.code, &e.message).await?,
        }
    }
    Ok(())
}

async fn dispatch(method: &str, params: Value) -> std::result::Result<Value, RpcError> {
    match method {
        "initialize" => Ok(json!({
            "protocolVersion": PROTOCOL_VERSION,
            "capabilities": { "tools": {} },
            "serverInfo": { "name": "notakase", "version": env!("CARGO_PKG_VERSION") },
        })),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({ "tools": tools::list() })),
        "tools/call" => {
            let name = params.get("name").and_then(Value::as_str).unwrap_or("");
            let args = params.get("arguments").cloned().unwrap_or(json!({}));
            match tools::call(name, args).await {
                Ok(text) => Ok(json!({ "content": [ { "type": "text", "text": text } ] })),
                Err(tools::ToolError::Unknown) => Err(rpc_err(-32602, format!("unknown tool: {name}"))),
                Err(tools::ToolError::Failed(msg)) => Ok(json!({
                    "content": [ { "type": "text", "text": msg } ],
                    "isError": true,
                })),
            }
        }
        "resources/list" => Ok(json!({ "resources": [] })),
        "prompts/list" => Ok(json!({ "prompts": [] })),
        other => Err(rpc_err(-32601, format!("method not found: {other}"))),
    }
}

async fn write_result<W: AsyncWriteExt + Unpin>(w: &mut W, id: Value, result: Value) -> Result<()> {
    write_frame(w, json!({ "jsonrpc": "2.0", "id": id, "result": result })).await
}

async fn write_error<W: AsyncWriteExt + Unpin>(w: &mut W, id: Value, code: i64, message: &str) -> Result<()> {
    write_frame(w, json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })).await
}

async fn write_frame<W: AsyncWriteExt + Unpin>(w: &mut W, v: Value) -> Result<()> {
    let mut line = serde_json::to_string(&v)?;
    line.push('\n');
    w.write_all(line.as_bytes()).await?;
    w.flush().await?;
    Ok(())
}
