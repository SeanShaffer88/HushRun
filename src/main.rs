mod app;
mod cli;
mod platform;
mod session;

use std::process::ExitCode;

fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    match cli::parse(&argv) {
        Ok(cli::Command::Help) => {
            print!("{}", cli::HELP);
            ExitCode::SUCCESS
        }
        Ok(cli::Command::Version) => {
            println!("HushRun {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        Ok(cmd) => match app::execute(cmd) {
            Ok(message) => {
                println!("{message}");
                ExitCode::SUCCESS
            }
            Err(message) => {
                eprintln!("hushrun: {message}");
                ExitCode::from(1)
            }
        },
        Err(message) => {
            eprintln!("hushrun: {message}\n\nRun `hushrun help` for usage.");
            ExitCode::from(2)
        }
    }
}
