//! herdr-smooth-scroll: eased, line-by-line scrollback scrolling for Herdr panes.
//!
//! A [Herdr](https://herdr.dev) plugin that ports [`azorng/tmux-smooth-scroll`] to Herdr's socket
//! API. Instead of jumping a whole page at once, each bound action moves the focused pane's
//! scrollback one line at a time, with the same per-line delay and easing math as the tmux
//! plugin's `animator.pl`.
//!
//! Herdr's mouse wheel and copy-mode scrolling live in the client and cannot be rebound by a
//! plugin, so this plugin ships its own actions (`scroll-*`, `halfpage-*`, `page-*`) for users to
//! bind in `config.toml`. The only pane-touching API calls it makes are `pane.get` and
//! `pane.scroll`.
//!
//! [`azorng/tmux-smooth-scroll`]: https://github.com/azorng/tmux-smooth-scroll

pub mod animator;
pub mod config;
pub mod herdr_client;
