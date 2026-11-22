//! chaus entry point. CLI parsing happens before GPUI and the GPU initialize.

use chaus_app::app;

fn main() {
    let command = match chaus_cli::parse(std::env::args_os().skip(1)) {
        Ok(command) => command,
        Err(error) => {
            eprintln!("chaus: {error}");
            eprintln!("Try 'chaus --help' for usage.");
            std::process::exit(2);
        }
    };
    match command {
        chaus_cli::Command::Help => print!("{}", chaus_cli::help_text()),
        chaus_cli::Command::Version => println!("chaus {}", env!("CARGO_PKG_VERSION")),
        chaus_cli::Command::Run { paths, new_window } => {
            let code = app::run(paths, new_window);
            if code != 0 {
                std::process::exit(code);
            }
        }
        _ => {}
    }
}
