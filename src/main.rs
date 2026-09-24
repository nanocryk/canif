use std::io::Write;

use clap::Parser;

use canif::cli::{self, Cli};

const CANIF_ERROR_CODE: i32 = 2;

fn main() {
    let code = match cli::execute(Cli::parse()) {
        Ok(code) => code,
        Err(error) => {
            eprintln!("canif: {error}");
            CANIF_ERROR_CODE
        }
    };
    let _ = std::io::stdout().flush();
    std::process::exit(code);
}
