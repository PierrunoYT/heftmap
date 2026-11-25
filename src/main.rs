// Release builds are GUI-subsystem on Windows (no console window); report mode re-attaches.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod app;
mod elevation;
mod format;
mod listing;
mod report;
mod scan;
mod tree;
mod treemap;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

const USAGE: &str = "\
Heftmap - disk usage analyzer

Usage:
  heftmap [PATH]                                     open the GUI (and scan PATH)
  heftmap --report PATH [--depth N] [--top N]        print a size-sorted tree

Options:
  --depth N   levels to print below PATH (default 2)
  --top N     largest entries to print per folder (default 10)";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("--report") => {
            attach_console();
            report::run(&args[1..])
        }
        Some("-h" | "--help") => {
            attach_console();
            println!("{USAGE}");
            ExitCode::SUCCESS
        }
        _ if args.len() > 1 => {
            attach_console();
            eprintln!("error: expected at most one PATH\n\n{USAGE}");
            ExitCode::from(2)
        }
        first => match app::run(first.map(PathBuf::from)) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                attach_console();
                eprintln!("error: {e}");
                ExitCode::FAILURE
            }
        },
    }
}

/// Absolute path; a bare Windows drive like `C:` means its root, not its current directory.
pub fn normalize_path(path: &Path) -> PathBuf {
    let bare_drive = cfg!(windows) && path.to_str().is_some_and(|s| s.len() == 2 && s.ends_with(':'));
    let path = if bare_drive { PathBuf::from(format!("{}\\", path.display())) } else { path.to_path_buf() };
    std::path::absolute(&path).unwrap_or(path)
}

/// Lets a GUI-subsystem build print to the console it was started from.
fn attach_console() {
    #[cfg(windows)]
    // SAFETY: plain Win32 call without pointers; failure (no parent console) is harmless.
    unsafe {
        windows_sys::Win32::System::Console::AttachConsole(
            windows_sys::Win32::System::Console::ATTACH_PARENT_PROCESS,
        );
    }
}
