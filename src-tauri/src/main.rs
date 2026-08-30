// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // `koden cli ...` is the in-process client for the CLI socket: answer and
    // exit before any Tauri or platform setup runs.
    if std::env::args().nth(1).as_deref() == Some("cli") {
        let code = koden_lib::modules::cli::client::run(std::env::args().skip(2).collect());
        std::process::exit(code);
    }

    #[cfg(target_os = "macos")]
    {
        // Disable macOS press-and-hold character popup, so key repeat works in terminal.
        use objc2::msg_send;
        use objc2_foundation::{ns_string, NSUserDefaults};
        unsafe {
            let defaults = NSUserDefaults::standardUserDefaults();
            let key = ns_string!("ApplePressAndHoldEnabled");
            let _: () = msg_send![&defaults, setBool: false, forKey: key];
        }
    }

    koden_lib::run()
}
