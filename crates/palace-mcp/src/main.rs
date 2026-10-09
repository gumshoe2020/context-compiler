#![forbid(unsafe_code)]
mod child;
mod diary_tool;
mod json_input;
mod kg_tools;
mod options;
mod reply_tool;
mod service;
use palace_store::Error;
use serde_json::{Value, json};
use service::Service;
use std::io::{self, BufRead, Write};
fn main() {
    // Even panic diagnostics must not leak into the gateway's stderr log.
    std::panic::set_hook(Box::new(|_| {}));
    if run().is_err() {
        std::process::exit(1);
    }
}
fn run() -> Result<(), Error> {
    let options = options::Options::parse(std::env::args_os().skip(1))?;
    let (person, assistant) = &options.names;
    palace_core::names::set(person.as_deref(), assistant.as_deref())?;
    let mut service = Service::open(options);
    let input = io::stdin();
    let mut output = io::stdout().lock();
    for line in input.lock().lines() {
        if let Some(response) = respond(&mut service, &line?) {
            serde_json::to_writer(&mut output, &response)?;
            output.write_all(b"\n")?;
            output.flush()?;
        }
    }
    Ok(())
}
fn error(id: Value, code: i32, message: &str) -> Value {
    json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":message}})
}
fn respond(service: &mut Service, line: &str) -> Option<Value> {
    let request: Value = match json_input::parse(line) {
        Ok(value) => value,
        Err(_) => return Some(error(Value::Null, -32700, "parse error")),
    };
    let id = request.get("id").cloned();
    let valid_id = id
        .as_ref()
        .is_none_or(|v| v.is_null() || v.is_string() || v.is_number());
    if request["jsonrpc"] != "2.0" || !request["method"].is_string() || !valid_id {
        return Some(error(Value::Null, -32600, "invalid request"));
    }
    let id = id?; // Notifications never receive responses, including initialized.
    let result = match request["method"].as_str().unwrap_or_default() {
        "initialize" => Ok(
            json!({"protocolVersion":"2024-11-05","capabilities":{"tools":{}},"serverInfo":{"name":"palace-mcp","version":env!("CARGO_PKG_VERSION")}}),
        ),
        "tools/list" => Ok(service.tools()),
        "tools/call" => service.call(&request["params"]),
        _ => return Some(error(id, -32601, "method not found")),
    };
    Some(match result.map_err(palace_store::busy::named) {
        Ok(result) => json!({"jsonrpc":"2.0","id":id,"result":result}),
        Err(failure) if palace_store::busy::Busy::is(&failure) => retryable(id, &failure),
        Err(failure) => error(id, -32602, &failure.to_string()),
    })
}

/// M5a item 3 (review S-1): a database held past the busy timeout is a server error the caller
/// may retry, not "invalid params".
pub const RETRY_LATER: i32 = -32001;

fn retryable(id: Value, failure: &Error) -> Value {
    json!({"jsonrpc":"2.0","id":id,"error":{"code":RETRY_LATER,"message":failure.to_string(),"data":{"retryable":true}}})
}
