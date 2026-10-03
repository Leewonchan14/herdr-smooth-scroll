# Repository Guidelines

`herdr-smooth-scroll` is a Herdr plugin: six manifest actions that animate the focused pane's
scrollback one line at a time through the public socket API, ported from
`azorng/tmux-smooth-scroll`'s `animator.pl`.

## Non-negotiable constraints

- **The only pane-touching calls are `pane.get` (read scroll metrics) and `pane.scroll` (move one
  line).** Never `pane.send_keys` / `pane.send_text`; never write to a pane's PTY.
- **The mouse wheel and copy-mode keys live in the client and cannot be rebound by a plugin.** Do
  not add code that pretends to; the README explains this and ships bindable actions instead.
- The delay/easing math must stay a faithful port of the tmux plugin: base delay
  `1000 + speed × 90` µs, scrolls under 10 lines stretched up to 3x, sine 0.3x-3x / quad 0.2x-3x /
  linear constant. `src/animator.rs` tests pin these numbers.
- Actions are static manifest entries (plugin v1 has no runtime action registration). Keep
  `herdr-plugin.toml`, the README action table, and `scripts/lab-e2e.py` in sync on action ids.
- Bindings documented in the README must be ones that actually fire in a live client. The validated
  set is `prefix+ctrl+b/f` (page), `prefix+ctrl+u/d` (halfpage), `prefix+alt+u/d` (3 lines).
  `prefix+PageUp`/`PageDown` never reach prefix mode (the client consumes page keys).

## Project shape

- `src/main.rs` — argv parsing, pane resolution (context JSON -> `HERDR_PANE_ID` -> `pane.current`),
  the step loop, and the state log line.
- `src/animator.rs` — pure math: lines per kind, edge stopping, the delay schedule. The module the
  tests lean on hardest.
- `src/config.rs` — `$HERDR_PLUGIN_CONFIG_DIR/config.toml`; option names mirror the tmux plugin's
  `@smooth-scroll-*` options.
- `src/herdr_client.rs` — Unix-socket JSON-RPC. The server answers one request per connection and
  closes it, so every call opens a fresh stream; covered with `UnixListener` fixtures.
- `scripts/lab-e2e.py` — the isolated end-to-end proof. Named non-default session only. `pyte` is
  optional; without it the rendered-frame assertions are skipped.

## Development process

TDD for behaviour changes: pin the math in `animator`/`config` tests first, then change the code.
Keep socket request shaping covered in `herdr_client` tests.

```bash
cargo fmt -- --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo build --release --locked
```

Local Herdr loop:

```bash
cargo build --release --locked
herdr plugin link .
herdr server reload-config
herdr plugin action invoke halfpage-up --plugin Leewonchan14.herdr-smooth-scroll
```

End-to-end lab (never the default session):

```bash
python3 scripts/lab-e2e.py
```

## Notes

- Do not commit `target/`, logs, or local editor files.
- Smoothness is only provable end-to-end: assert rendered frames and sampled offsets in the lab, not
  just the delay arithmetic.
- Update this file only for durable repository-wide guidance.
