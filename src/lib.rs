//! The engine of [Agent Center](https://github.com/alexandreravelli/agent-center): the app
//! catalog, the version checks, and the scripts that install, update and remove apps.
//!
//! The `agent-center` program (a GTK 3 window and a command line) is built on this library.
//!
//! - [`catalog`] reads the apps from TOML files and decides which ones may run as root.
//! - [`versions`] finds the installed and the latest version of an app.
//! - [`runner`] runs the scripts, as the user or as root through sudo or pkexec.

pub mod catalog;
pub mod runner;
pub mod versions;
