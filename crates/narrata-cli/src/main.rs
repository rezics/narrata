#![forbid(unsafe_code)]

mod commands;

fn main() {
    if let Err(error) = commands::run(std::env::args().skip(1).collect()) {
        eprintln!("{error}");
        std::process::exit(2);
    }
}
