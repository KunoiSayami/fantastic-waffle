#![feature(async_closure)]
#![feature(result_option_inspect)]

mod configure;
mod database;
mod file;
mod server;

use crate::configure::current::Configure;
use crate::file::{FileDaemon, FileWatcher};
use crate::server::{WebServer, DEFAULT_WAIT_TIME};
use clap::{arg, command};
use std::sync::Arc;
use tokio::sync::RwLock;

const DEFAULT_CONFIGURE_FILE: &str = "config.toml";

async fn async_main(
    config_path: String,
    host: Option<&String>,
    port: Option<&u16>,
    skip_check: bool,
) -> anyhow::Result<()> {
    let config = Configure::load(config_path).await?;
    let bind = config.parse_host_and_port(host, port);
    let user_pool = Arc::new(RwLock::new(config.build_hashmap()));

    //let web_server = WebServer::router_start(bind, user_pool.clone());
    Ok(())
}

fn main() -> anyhow::Result<()> {
    let matches = command!()
        .args(&[
            arg!(-c --config [CONFIGURE_FILE] "Specify configure file location")
                .default_value(DEFAULT_CONFIGURE_FILE),
            arg!(-l --listen [HOST] "Override server listen host"),
            arg!(-p --port [PORT] "Override server port"),
            arg!(--"skip-check" "Skip check existing files"),
            arg!(--"server-timeout" "Override sever request timeout, if set more than 3, it will always set as 3")
                .default_value(&*format!("{}", DEFAULT_WAIT_TIME)),
        ])
        .get_matches();
    env_logger::Builder::from_default_env().init();

    server::WAIT_TIME
        .set({
            let set_time = *matches.get_one::<u64>("server-timeout").unwrap();
            if set_time > DEFAULT_WAIT_TIME {
                DEFAULT_WAIT_TIME
            } else {
                set_time
            }
        })
        .unwrap();

    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async_main(
            matches
                .get_one::<String>("CONFIGURE_FILE")
                .unwrap()
                .to_string(),
            matches.get_one::<String>("HOST"),
            matches.get_one::<u16>("PORT"),
            matches.get_flag("skip-check"),
        ))?;
    Ok(())
}
