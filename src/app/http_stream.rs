use super::{App, http::CORS_HEADERS};
use anyhow::Result;
use serde_json::Value;
use std::io::Write;
use std::net::TcpStream;

impl App {
    /// Live Server-Sent Events stream: replays recent events, then follows
    /// the events table until the client disconnects.
    pub(super) fn stream_events(&self, mut stream: TcpStream) -> Result<()> {
        // Streaming writes are spaced out; the per-connection write timeout
        // only needs to cover a single event or heartbeat write.
        let _ = stream.set_write_timeout(Some(std::time::Duration::from_secs(10)));
        write!(
            stream,
            "HTTP/1.1 200 OK\r\n{CORS_HEADERS}content-type: text/event-stream\r\ncache-control: no-cache\r\nconnection: close\r\n\r\n"
        )?;
        write!(stream, "retry: 5000\n\n")?;
        let mut last_id: i64 = 0;
        let replay = self.list_events(None, 100)?;
        if replay.is_empty() {
            write!(stream, "event: ready\ndata: {{\"ok\":true}}\n\n")?;
        }
        for event in replay.into_iter().rev() {
            last_id = write_sse_event(&mut stream, &event)?.max(last_id);
        }
        stream.flush()?;
        let mut idle_polls = 0u32;
        loop {
            let fresh = self.events_after(last_id)?;
            if fresh.is_empty() {
                idle_polls += 1;
                // Heartbeat comment roughly every 5 seconds keeps proxies and
                // clients aware the stream is alive and detects disconnects.
                if idle_polls >= 10 {
                    idle_polls = 0;
                    if write!(stream, ": keepalive\n\n").is_err() {
                        return Ok(());
                    }
                    if stream.flush().is_err() {
                        return Ok(());
                    }
                }
            } else {
                idle_polls = 0;
                for event in fresh {
                    match write_sse_event(&mut stream, &event) {
                        Ok(id) => last_id = id.max(last_id),
                        Err(_) => return Ok(()),
                    }
                }
                if stream.flush().is_err() {
                    return Ok(());
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(500));
        }
    }
}

fn write_sse_event(stream: &mut TcpStream, event: &Value) -> Result<i64> {
    let id = event.get("id").and_then(Value::as_i64).unwrap_or(0);
    write!(
        stream,
        "id: {id}\nevent: {}\ndata: {}\n\n",
        event
            .get("event_type")
            .and_then(Value::as_str)
            .unwrap_or("planr_event"),
        serde_json::to_string(event)?
    )?;
    Ok(id)
}
