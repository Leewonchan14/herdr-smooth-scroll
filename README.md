# herdr-smooth-scroll

`prefix+ctrl+b` → **eased, line-by-line scrollback scrolling for the focused Herdr pane**: instead
of teleporting a whole page, the pane walks one line at a time with configurable speed and easing.

A [Herdr](https://herdr.dev) plugin. MIT.

## Why this exists (the problem it solves)

Herdr scrolls instantly: the mouse wheel jumps `ui.mouse_scroll_lines` (default 3) per notch, and
copy mode's `PageUp` / `ctrl+u` jumps half or full pages. [`azorng/tmux-smooth-scroll`] fixes this
in tmux by rebinding the copy-mode scroll keys to an animated loop.

A Herdr plugin cannot rebind those keys: the wheel and copy mode live in the **client**, not in
anything the plugin API exposes (the same constraint that shapes
[`Leewonchan14/herdr-flash`](https://github.com/Leewonchan14/herdr-flash)). So this plugin does the
next best thing: six actions that animate the real pane through the public socket API
(`pane.get` + `pane.scroll`), using the same per-line delay and easing math as the tmux plugin's
`animator.pl`. Bind them, and long jumps read as motion instead of teleporting.

[`azorng/tmux-smooth-scroll`]: https://github.com/azorng/tmux-smooth-scroll

## Install

```bash
herdr plugin install Leewonchan14/herdr-smooth-scroll   # the Herdr server runs the build: cargo must be on its PATH
herdr server reload-config
```

Local development install (no build step run by Herdr):

```bash
cargo build --release --locked
herdr plugin link .
herdr server reload-config
```

## Bindings

Add to `~/.config/herdr/config.toml`. These six were validated against a live Herdr client;
`prefix` is whatever your config sets (the examples use the default `ctrl+b`).

```toml
[[keys.command]]
key = "prefix+ctrl+b"
type = "plugin_action"
command = "Leewonchan14.herdr-smooth-scroll.page-up"
description = "Smooth page up"

[[keys.command]]
key = "prefix+ctrl+f"
type = "plugin_action"
command = "Leewonchan14.herdr-smooth-scroll.page-down"
description = "Smooth page down"

[[keys.command]]
key = "prefix+ctrl+u"
type = "plugin_action"
command = "Leewonchan14.herdr-smooth-scroll.halfpage-up"
description = "Smooth half-page up"

[[keys.command]]
key = "prefix+ctrl+d"
type = "plugin_action"
command = "Leewonchan14.herdr-smooth-scroll.halfpage-down"
description = "Smooth half-page down"

[[keys.command]]
key = "prefix+alt+u"
type = "plugin_action"
command = "Leewonchan14.herdr-smooth-scroll.scroll-up"
description = "Smooth scroll up"

[[keys.command]]
key = "prefix+alt+d"
type = "plugin_action"
command = "Leewonchan14.herdr-smooth-scroll.scroll-down"
description = "Smooth scroll down"
```

Check `prefix+?` first: pick keys that are still free in your config. `prefix+PageUp`/`PageDown` do
**not** work — the client consumes page keys before prefix mode sees them. `prefix+alt+...` works in
Herdr's own input path but depends on your terminal reporting Alt.

## Actions

| Action id | Suggested key | Moves |
|---|---|---|
| `scroll-up` / `scroll-down` | `prefix+alt+u` / `prefix+alt+d` | 3 lines (configurable) |
| `halfpage-up` / `halfpage-down` | `prefix+ctrl+u` / `prefix+ctrl+d` | half the pane height |
| `page-up` / `page-down` | `prefix+ctrl+b` / `prefix+ctrl+f` | one pane height |

Every action stops at the scrollback edges and never writes to the pane's PTY. Scrolling down to
offset 0 returns the pane to the live tail.

## Configuration

`herdr plugin config-dir Leewonchan14.herdr-smooth-scroll` prints the directory; create
`config.toml` there:

```toml
speed = 100            # 0-1000, lower is faster (100 -> 10 ms per line)
easing = "sine"        # linear | sine | quad
normal_lines = 3       # lines per scroll-up / scroll-down
halfpage_lines = 19    # omit for half the pane height
fullpage_lines = 39    # omit for one pane height
```

The option names and semantics mirror the tmux plugin's options:

| `azorng/tmux-smooth-scroll` | `herdr-smooth-scroll` |
|---|---|
| `@smooth-scroll-speed` (0-1000, lower = faster) | `speed` |
| `@smooth-scroll-easing` (`linear`/`sine`/`quad`) | `easing` |
| `@smooth-scroll-normal` (default 3) | `normal_lines` |
| `@smooth-scroll-halfpage` (default pane height / 2) | `halfpage_lines` (default viewport / 2) |
| `@smooth-scroll-fullpage` (default pane height) | `fullpage_lines` (default viewport) |
| `@smooth-scroll-mouse` | — the wheel is client-owned; a plugin cannot animate it |
| `@smooth-scroll-exit-copy-mode-at-bottom` | — this plugin never enters copy mode |

## How it works

1. `pane.get` reads the pane's scroll metrics (`offset_from_bottom`, `max_offset_from_bottom`,
   `viewport_rows`).
2. Each step calls `pane.scroll` with the offset moved by exactly one line, and uses the response as
   the base for the next step — so rapid repeated presses compose (each contributes its own lines)
   instead of fighting over an absolute target.
3. Between lines the plugin sleeps `(1000 + speed × 90) µs × stretch ÷ velocity(t)` — a direct port
   of the tmux plugin's `animator.pl`: scrolls shorter than 10 lines are stretched up to 3x, and the
   easing curve scales the velocity (sine 0.3x-3x, quad 0.2x-3x, linear constant).
4. The server clamps every offset, so edge handling falls out of the response: at the top or bottom
   the loop simply stops.

The client renders these updates as ordinary pane redraws; the e2e lab observes ~10 distinct
rendered frames for a 19-line halfpage on a 40-row terminal.

## Limitations

- **The mouse wheel and copy-mode keys cannot be smoothed.** They are handled by the client, and
  plugin v1 has no hook for them. This plugin gives you bindable actions instead.
- Requires Herdr **0.9+** (the `pane.scroll` socket method; it is not exposed in the CLI).
- The scroll position is pane state, shared by every client attached to that pane. New output does
  not reset it.
- `linux` and `macos` only: the client speaks to the Herdr Unix socket directly.

## Lineage and credits

- The animation math — per-line delay, short-scroll stretch, easing curves — is a direct port of
  [`azorng/tmux-smooth-scroll`] (MIT, Copyright (c) 2025 azorng), including its option names.
- The repository layout, socket-client conventions, and the isolated PTY lab harness follow
  [`Leewonchan14/herdr-flash`](https://github.com/Leewonchan14/herdr-flash).

## Development

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
herdr plugin action invoke halfpage-up --plugin Leewonchan14.herdr-smooth-scroll
```

End-to-end proof in an isolated named session (never the default session):

```bash
python3 scripts/lab-e2e.py
```

It fills a pane with 300 numbered lines, invokes `halfpage-up`, samples the scroll offset, and
reconstructs rendered client frames. `pip install pyte` enables the rendered-frame assertions; the
offset and final-state assertions run without it.

## 한국어 요약

- Herdr은 스크롤이 즉시 점프합니다(휠은 `ui.mouse_scroll_lines`=3줄, copy mode는 페이지 단위).
  이 플러그인은 `azorng/tmux-smooth-scroll`을 Herdr로 옮긴 것으로, **한 줄씩 이징을 걸어**
  스크롤합니다.
- 휠과 copy mode 키는 클라이언트 소유라 플러그인이 재바인딩할 수 없습니다(herdr-flash가 copy mode
  커서를 옮길 수 없었던 것과 같은 제약). 대신 6개의 액션(`scroll-*`, `halfpage-*`, `page-*`)을
  제공하고, `config.toml`에서 원하는 키에 바인딩합니다.
- 검증된 바인딩: `prefix+ctrl+b/f`(페이지), `prefix+ctrl+u/d`(하프페이지), `prefix+alt+u/d`(3줄).
  `prefix+PageUp/PageDown`은 동작하지 않습니다(클라이언트가 페이지 키를 먼저 소비).
- 옵션은 tmux 원본과 동일한 의미입니다: `speed`(0-1000, 낮을수록 빠름), `easing`(linear/sine/quad),
  `normal_lines`/`halfpage_lines`/`fullpage_lines`.
- 내부 동작: `pane.get`으로 위치를 읽고 `pane.scroll`로 한 줄씩 이동하며, 줄 사이 지연은 원본
  `animator.pl` 공식 그대로입니다. 각 단계가 응답을 다음 기준으로 삼아 연타해도 합산됩니다.

## License

MIT — see [LICENSE](LICENSE) for the third-party lineage notes.
