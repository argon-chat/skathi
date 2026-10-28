//! vn-desktop — the desktop surface: the argv parser, the shell, single
//! instance and custom protocols (docs/SHELL.md, docs/LAUNCH.md).
//!
//! This first module is the precondition for everything else here
//! (research/reports/m25/desktop-apis-design.md finding 6): one parser for this
//! process's command line, honoured by every consumer.

pub mod args;
