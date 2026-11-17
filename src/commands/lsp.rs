use std::error::Error;

use lsp_server::{Connection, ErrorCode, Message, Response};
use oi::Reported;
use oi::driver::fail;
use serde_json::json;

/// Serve the language server over stdio.
pub fn serve() -> Result<(), Reported> {
	run().map_err(|e| fail(format!("lsp: {e}")))
}

fn run() -> Result<(), Box<dyn Error + Sync + Send>> {
	let (conn, io) = Connection::stdio();
	conn.initialize(json!({}))?;

	for msg in &conn.receiver {
		if let Message::Request(req) = msg {
			if conn.handle_shutdown(&req)? {
				break;
			}
			let resp = Response::new_err(req.id, ErrorCode::MethodNotFound as i32, req.method);
			conn.sender.send(resp.into())?;
		}
	}

	drop(conn);
	io.join()?;
	Ok(())
}
