// On Windows no console opens when the game is started from the file manager; a command typed in
// a terminal writes to that terminal's console (`console::attach`).
#![cfg_attr(all(windows, not(test)), windows_subsystem = "windows")]
#![cfg_attr(all(target_os = "ios", not(test)), no_main)]

mod audio;
mod boot;
mod cli;
#[cfg(windows)]
mod console;
mod findcmd;
mod input;
mod layout;
mod probe;
mod run;
mod simcmd;
mod textcmd;
mod window;

use std::ffi::OsString;
use std::io::{self, Write};
#[cfg(not(target_os = "ios"))]
use std::process::ExitCode;

use sdl3::messagebox::{MessageBoxFlag, show_simple_message_box};

use cli::{CliError, USAGE};

#[cfg(not(target_os = "ios"))]
fn main() -> ExitCode {
    #[cfg(windows)]
    console::attach();
    ExitCode::from(start(std::env::args_os().skip(1).collect()))
}

fn start(args: Vec<OsString>) -> u8 {
    let boxed = in_a_box(&args);
    match cli::run(args) {
        Ok(()) => 0,
        Err(CliError::Usage) => {
            // A closed standard error is not worth a panic, which `eprintln!` would raise.
            let _ = writeln!(io::stderr(), "{USAGE}");
            2
        }
        Err(CliError::Failed(err)) => {
            let _ = writeln!(io::stderr(), "rustyzoo: {err}");
            if boxed {
                show_box(&err.to_string());
            }
            1
        }
    }
}

#[cfg(all(target_os = "ios", not(test)))]
#[unsafe(export_name = "main")]
unsafe extern "C" fn ipad_main(
    argc: std::ffi::c_int,
    argv: *mut *mut std::ffi::c_char,
) -> std::ffi::c_int {
    // SAFETY: argc and argv are the original process arguments; SDL owns the UIKit bootstrap.
    unsafe { sdl3::sys::main::SDL_RunApp(argc, argv, Some(ipad_run), std::ptr::null_mut()) }
}

#[cfg(all(target_os = "ios", not(test)))]
extern "C" fn ipad_run(_: std::ffi::c_int, _: *mut *mut std::ffi::c_char) -> std::ffi::c_int {
    sdl3::hint::set("SDL_TOUCH_MOUSE_EVENTS", "0");
    sdl3::hint::set("SDL_MOUSE_TOUCH_EVENTS", "0");
    // Route Apple Pencil through the same gestures as a finger. SDL otherwise
    // sends a second, synthetic mouse click for the same contact.
    sdl3::hint::set("SDL_PEN_TOUCH_EVENTS", "1");
    sdl3::hint::set("SDL_PEN_MOUSE_EVENTS", "0");
    let documents = match sdl3::filesystem::get_user_folder(sdl3::filesystem::Folder::DOCUMENTS) {
        Ok(dir) => dir,
        Err(error) => {
            show_box(&error.to_string());
            return 1;
        }
    };
    const GAME_FOLDER: &str = "Zoo-Tycoon-Complete-Collection";
    let data = documents.join(GAME_FOLDER);
    let data = if data.is_dir() {
        data
    } else {
        match sdl3::filesystem::get_base_path() {
            Ok(bundle) if bundle.join(GAME_FOLDER).is_dir() => bundle.join(GAME_FOLDER),
            _ => {
                show_box(
                    "Copy the Zoo-Tycoon-Complete-Collection folder to RustyZoo in Files, then reopen the app.",
                );
                return 1;
            }
        }
    };
    i32::from(start(vec![
        "--data".into(),
        data.into_os_string(),
        "run".into(),
    ]))
}

/// Whether an error is also shown in a message box: a program opened from the file manager, with
/// no arguments, has no visible error output.
fn in_a_box(args: &[OsString]) -> bool {
    cfg!(target_os = "ios") || args.is_empty()
}

fn show_box(message: &str) {
    if let Err(e) = show_simple_message_box(MessageBoxFlag::ERROR, "RustyZoo", message, None) {
        let _ = writeln!(io::stderr(), "rustyzoo: warning: no message box: {e}");
    }
}

#[cfg(all(test, not(target_os = "ios")))]
mod tests {
    use super::in_a_box;

    #[test]
    fn only_a_start_without_arguments_shows_errors_in_a_box() {
        assert!(in_a_box(&[]));
        assert!(!in_a_box(&["run".into()]));
        assert!(!in_a_box(&["--help".into()]));
        assert!(!in_a_box(&["Saved Games/a.zoo".into()]));
    }
}
