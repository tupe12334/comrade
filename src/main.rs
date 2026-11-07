//! Command-line entry point. All logic lives in the `comrade` library.

fn main() {
    std::process::exit(comrade::cli::run(std::env::args_os()));
}
