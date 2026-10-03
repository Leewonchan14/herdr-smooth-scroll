//! Herdr Unix-socket JSON-RPC client.
//!
//! The plugin moves panes only through `pane.scroll`, and only after reading the current position
//! with `pane.get`. Nothing here writes to a pane's PTY or sends key presses. The server answers
//! one request per connection, so every call opens a fresh stream.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{bail, Context, Result};
use serde_json::{json, Value};

const RPC_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScrollState {
    pub offset_from_bottom: u64,
    pub max_offset_from_bottom: u64,
    pub viewport_rows: u64,
}

#[derive(Debug)]
pub struct SocketClient {
    socket_path: PathBuf,
    next_id: u64,
}

impl SocketClient {
    pub fn connect(socket_path: &Path) -> Result<Self> {
        // Fail fast when the socket is unusable, but keep the path for per-call connections.
        UnixStream::connect(socket_path).with_context(|| {
            format!(
                "cannot connect to the Herdr API socket at {}",
                socket_path.display()
            )
        })?;
        Ok(Self {
            socket_path: socket_path.to_path_buf(),
            next_id: 1,
        })
    }

    fn call(&mut self, method: &str, params: Value) -> Result<Value> {
        let id = format!("herdr-smooth-scroll:{}", self.next_id);
        self.next_id += 1;
        let request = json!({ "id": id, "method": method, "params": params });
        let mut stream = UnixStream::connect(&self.socket_path).with_context(|| {
            format!(
                "cannot connect to the Herdr API socket at {}",
                self.socket_path.display()
            )
        })?;
        stream
            .set_read_timeout(Some(RPC_TIMEOUT))
            .context("failed to set socket read timeout")?;
        stream
            .set_write_timeout(Some(RPC_TIMEOUT))
            .context("failed to set socket write timeout")?;
        let mut payload = serde_json::to_string(&request)?;
        payload.push('\n');
        stream
            .write_all(payload.as_bytes())
            .context("failed to write the Herdr request")?;
        stream
            .flush()
            .context("failed to flush the Herdr request")?;

        let mut line = String::new();
        BufReader::new(stream)
            .read_line(&mut line)
            .with_context(|| format!("failed to read the {method} response"))?;
        if line.trim().is_empty() {
            bail!("{method} returned no response");
        }
        let response: Value = serde_json::from_str(&line)
            .with_context(|| format!("{method} returned invalid JSON"))?;
        if let Some(error) = response.get("error") {
            let code = error["code"].as_str().unwrap_or("unknown");
            let message = error["message"].as_str().unwrap_or("no message");
            bail!("Herdr API error {code}: {message}");
        }
        response
            .get("result")
            .cloned()
            .with_context(|| format!("{method} returned no result"))
    }

    /// Focused pane from `pane.current`, used when the plugin context has no pane.
    pub fn focused_pane_id(&mut self) -> Result<String> {
        let result = self.call("pane.current", json!({}))?;
        result["pane"]["pane_id"]
            .as_str()
            .map(str::to_string)
            .context("pane.current did not include a pane id")
    }

    /// Current scroll metrics for a pane, from `pane.get`.
    pub fn scroll_state(&mut self, pane_id: &str) -> Result<ScrollState> {
        let result = self.call("pane.get", json!({ "pane_id": pane_id }))?;
        parse_scroll(&result, "pane.get")
    }

    /// Move a pane to an absolute scroll position, from `pane.scroll`. The server clamps the
    /// offset, so the returned state is authoritative.
    pub fn scroll_to(&mut self, pane_id: &str, offset_from_bottom: u64) -> Result<ScrollState> {
        let result = self.call(
            "pane.scroll",
            json!({ "pane_id": pane_id, "offset_from_bottom": offset_from_bottom }),
        )?;
        parse_scroll(&result, "pane.scroll")
    }
}

fn parse_scroll(result: &Value, method: &str) -> Result<ScrollState> {
    let actual = result["type"].as_str().unwrap_or("<missing>");
    if actual != "pane_info" {
        bail!("expected a pane_info result from {method}, got {actual}");
    }
    let scroll = &result["pane"]["scroll"];
    Ok(ScrollState {
        offset_from_bottom: scroll["offset_from_bottom"]
            .as_u64()
            .with_context(|| format!("{method} result did not include a scroll offset"))?,
        max_offset_from_bottom: scroll["max_offset_from_bottom"]
            .as_u64()
            .with_context(|| format!("{method} result did not include a scroll maximum"))?,
        viewport_rows: scroll["viewport_rows"]
            .as_u64()
            .with_context(|| format!("{method} result did not include the viewport rows"))?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::BufWriter;
    use std::os::unix::net::UnixListener;
    use std::thread;

    /// Serve one request and reply with `body`, returning the received request line.
    fn one_shot_server(body: &str) -> (PathBuf, thread::JoinHandle<String>) {
        let path = std::env::temp_dir().join(format!(
            "herdr-smooth-scroll-test-{}-{}.sock",
            std::process::id(),
            rand_suffix()
        ));
        let _ = std::fs::remove_file(&path);
        let listener = UnixListener::bind(&path).unwrap();
        let body = body.to_string();
        let handle = thread::spawn(move || {
            // `SocketClient::connect` probes the socket, so skip connections without a request.
            for _ in 0..2 {
                let (stream, _) = listener.accept().unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut request = String::new();
                if reader.read_line(&mut request).unwrap_or(0) == 0 {
                    continue;
                }
                let mut writer = BufWriter::new(stream);
                writer.write_all(body.as_bytes()).unwrap();
                writer.write_all(b"\n").unwrap();
                writer.flush().unwrap();
                return request;
            }
            panic!("no request reached the fixture server");
        });
        (path, handle)
    }

    fn rand_suffix() -> u64 {
        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
    }

    fn pane_info(offset: u64, max: u64, rows: u64) -> String {
        format!(
            r#"{{"id":"x","result":{{"type":"pane_info","pane":{{"pane_id":"w1:p1","scroll":{{"offset_from_bottom":{offset},"max_offset_from_bottom":{max},"viewport_rows":{rows}}}}}}}}}"#
        )
    }

    #[test]
    fn scroll_state_reads_the_metrics_from_pane_get() {
        let (path, server) = one_shot_server(&pane_info(12, 240, 30));
        let mut client = SocketClient::connect(&path).unwrap();
        let state = client.scroll_state("w1:p1").unwrap();
        assert_eq!(
            state,
            ScrollState {
                offset_from_bottom: 12,
                max_offset_from_bottom: 240,
                viewport_rows: 30
            }
        );
        let request: Value = serde_json::from_str(&server.join().unwrap()).unwrap();
        assert_eq!(request["method"], "pane.get");
        assert_eq!(request["params"]["pane_id"], "w1:p1");
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn scroll_to_shapes_pane_scroll_and_returns_the_clamped_state() {
        let (path, server) = one_shot_server(&pane_info(264, 264, 39));
        let mut client = SocketClient::connect(&path).unwrap();
        let state = client.scroll_to("w1:p1", 999_999).unwrap();
        assert_eq!(state.offset_from_bottom, 264);
        let request: Value = serde_json::from_str(&server.join().unwrap()).unwrap();
        assert_eq!(request["method"], "pane.scroll");
        assert_eq!(request["params"]["pane_id"], "w1:p1");
        assert_eq!(request["params"]["offset_from_bottom"], 999_999);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn focused_pane_id_reads_pane_current() {
        let body = r#"{"id":"x","result":{"type":"pane_current","pane":{"pane_id":"w2:p3"}}}"#;
        let (path, server) = one_shot_server(body);
        let mut client = SocketClient::connect(&path).unwrap();
        assert_eq!(client.focused_pane_id().unwrap(), "w2:p3");
        let request: Value = serde_json::from_str(&server.join().unwrap()).unwrap();
        assert_eq!(request["method"], "pane.current");
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn api_errors_surface_the_code_and_message() {
        let body =
            r#"{"id":"x","error":{"code":"pane_not_found","message":"pane w9:p9 not found"}}"#;
        let (path, server) = one_shot_server(body);
        let mut client = SocketClient::connect(&path).unwrap();
        let error = client.scroll_state("w9:p9").unwrap_err().to_string();
        assert!(error.contains("pane_not_found"), "{error}");
        assert!(error.contains("not found"), "{error}");
        let _ = server.join().unwrap();
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn a_pane_without_scroll_metrics_is_an_error() {
        let body = r#"{"id":"x","result":{"type":"pane_info","pane":{"pane_id":"w1:p1"}}}"#;
        let (path, server) = one_shot_server(body);
        let mut client = SocketClient::connect(&path).unwrap();
        let error = client.scroll_state("w1:p1").unwrap_err().to_string();
        assert!(error.contains("scroll offset"), "{error}");
        let _ = server.join().unwrap();
        let _ = std::fs::remove_file(path);
    }
}
