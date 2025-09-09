//! Build script generating manual pages from the CLI definition.

use std::{fs, path::PathBuf};

use clap::CommandFactory;
use clap_mangen::Man;

#[path = "src/cli.rs"]
mod cli;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("cargo:rerun-if-changed=src/cli.rs");

    let out_dir = PathBuf::from("target/generated-man");
    fs::create_dir_all(&out_dir)?;

    let cmd = cli::Cli::command();
    let man = Man::new(cmd);
    let mut buf: Vec<u8> = Vec::new();
    man.render(&mut buf)?;
    fs::write(out_dir.join("wireframe.1"), buf)?;

    Ok(())
}
