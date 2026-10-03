//! Entry point: animate one scroll gesture for the focused pane.
//!
//! Usage: `herdr-smooth-scroll <up|down> <normal|halfpage|fullpage>`
//!
//! Each step moves the pane one line with `pane.scroll`, pausing between lines with the delay
//! schedule from `azorng/tmux-smooth-scroll`'s animator. Because every step re-reads the pane's
//! post-move state from the `pane.scroll` response, overlapping invocations (rapid key presses)
//! compose: each contributes its own lines instead of fighting over an absolute target.

use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use herdr_smooth_scroll::animator::{self, Direction, ScrollKind};
use herdr_smooth_scroll::config;
use herdr_smooth_scroll::herdr_client::SocketClient;
use serde_json::Value;

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let (direction, kind) = match (args.next(), args.next()) {
        (Some(direction), Some(kind)) => (
            Direction::parse(&direction)
                .with_context(|| format!("unknown direction {direction:?}"))?,
            ScrollKind::parse(&kind).with_context(|| format!("unknown scroll kind {kind:?}"))?,
        ),
        _ => bail!("usage: herdr-smooth-scroll <up|down> <normal|halfpage|fullpage>"),
    };
    run(direction, kind)
}

fn run(direction: Direction, kind: ScrollKind) -> Result<()> {
    let socket_path = socket_path()?;
    let mut client = SocketClient::connect(&socket_path)?;
    let pane_id = resolve_pane_id(&mut client)?;
    let settings = config::load(config_dir().as_deref())?;

    let mut state = client.scroll_state(&pane_id)?;
    let start = state.offset_from_bottom;
    let viewport_rows = usize::try_from(state.viewport_rows).unwrap_or(0);
    let lines = animator::resolve_lines(kind, &settings, viewport_rows);
    let base_us = animator::base_delay_us(settings.speed);

    let mut steps = 0usize;
    let mut index = 0usize;
    while index < lines {
        let Some(next) = animator::next_offset(
            state.offset_from_bottom,
            state.max_offset_from_bottom,
            direction,
        ) else {
            break;
        };
        state = client.scroll_to(&pane_id, next)?;
        steps += 1;
        index += 1;
        let more_steps = index < lines
            && animator::next_offset(
                state.offset_from_bottom,
                state.max_offset_from_bottom,
                direction,
            )
            .is_some();
        if more_steps {
            let delay = animator::step_delay_us(base_us, lines, index - 1, settings.easing);
            thread::sleep(Duration::from_micros(delay));
        }
    }

    log_state(&format!(
        "pane={pane_id} dir={direction:?} kind={kind:?} lines={lines} steps={steps} \
         from={start} to={} max={} speed={} easing={:?}",
        state.offset_from_bottom, state.max_offset_from_bottom, settings.speed, settings.easing
    ));
    Ok(())
}

fn socket_path() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("HERDR_SOCKET_PATH") {
        return Ok(PathBuf::from(path));
    }
    let home = std::env::var_os("HOME").context("HOME is not set")?;
    let fallback = PathBuf::from(home).join(".config/herdr/herdr.sock");
    if fallback.exists() {
        return Ok(fallback);
    }
    bail!(
        "HERDR_SOCKET_PATH is not set and {} does not exist",
        fallback.display()
    )
}

fn config_dir() -> Option<PathBuf> {
    std::env::var_os("HERDR_PLUGIN_CONFIG_DIR").map(PathBuf::from)
}

/// Pane to scroll: the plugin context's focused pane, else the pane env var, else `pane.current`.
fn resolve_pane_id(client: &mut SocketClient) -> Result<String> {
    if let Some(context) = std::env::var_os("HERDR_PLUGIN_CONTEXT_JSON") {
        let parsed: Value = serde_json::from_str(&context.to_string_lossy())
            .context("HERDR_PLUGIN_CONTEXT_JSON is not valid JSON")?;
        if let Some(pane_id) = parsed["focused_pane_id"].as_str() {
            return Ok(pane_id.to_string());
        }
    }
    if let Some(pane_id) = std::env::var_os("HERDR_PANE_ID") {
        return Ok(pane_id.to_string_lossy().into_owned());
    }
    client.focused_pane_id()
}

/// Append a line to `$HERDR_PLUGIN_STATE_DIR/herdr-smooth-scroll.log` when Herdr provides one.
fn log_state(message: &str) {
    let Some(dir) = std::env::var_os("HERDR_PLUGIN_STATE_DIR") else {
        return;
    };
    let path = Path::new(&dir).join("herdr-smooth-scroll.log");
    let line = format!("{message}\n");
    let _ = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .and_then(|mut file| std::io::Write::write_all(&mut file, line.as_bytes()));
}
