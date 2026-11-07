//! Command-line entry point. All logic lives in the `modelgate` library.

fn main() {
    std::process::exit(modelgate::cli::run(std::env::args_os()));
}
