#![feature(async_closure)]
mod configure;
mod database;
mod file;
mod server;

use clap::{arg, command};

fn main() -> anyhow::Result<()> {
    let matches = command!()
        .args(&[
            arg!(-c --config [CONFIGURE_FILE] "Specify configure file location"),
            arg!(-l --listen [HOST] "Override server listen host"),
            arg!(-p --port [PORT] "Override server port"),
            arg!(--"skip-check" "Skip check existing files"),
        ])
        .get_matches();
    env_logger::Builder::from_default_env().init();
    Ok(())
}
