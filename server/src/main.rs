#![feature(async_closure)]
#![feature(result_option_inspect)]

mod configure;
mod database;
mod file;
mod server;

use crate::configure::current::Configure;
use crate::database::load_database;
use crate::file::{init_files, FileDaemon, FileWatcher};
use crate::server::{router_start, DEFAULT_WAIT_TIME, DEFAULT_WAIT_TIME_STR};
use anyhow::anyhow;
use clap::{arg, command};
use log::warn;
use publib::types::ExitExt;
use std::env;
use std::future::Future;
use std::sync::Arc;
use tap::TapOptional;
use tokio::sync::RwLock;

const DEFAULT_CONFIGURE_FILE: &str = "config.toml";

async fn wait_to_stop<Fut>(kill: impl FnOnce() -> Fut) -> !
where
    Fut: Future<Output = ()>,
{
    use log::{error, info, trace};
    tokio::signal::ctrl_c().await.unwrap();
    info!("Recv SIGINT, send signal to thread.");
    kill().await;
    trace!("Send signal!");
    tokio::signal::ctrl_c().await.unwrap();
    error!("Force exit program.");
    std::process::exit(137);
}

async fn async_main(
    config_path: String,
    host: Option<&String>,
    port: Option<&u16>,
    skip_check: bool,
) -> anyhow::Result<()> {
    let config = Configure::load(config_path.clone()).await?;

    let mut database = load_database(&config.database())
        .await
        .map_err(|e| anyhow!("Unable to load database: {:?}", e))?;

    env::set_current_dir(config.working_directory())
        .map_err(|e| anyhow!("Unable change directory: {:?}", e))?;

    let bind = config.parse_host_and_port(host, port);
    let user_pool = Arc::new(RwLock::new(config.build_hashmap()));

    if !skip_check {
        init_files(&mut database, ".")
            .await
            .map_err(|e| anyhow!("Init files failure: {:?}", e))?;
    }

    let (file_daemon, file_event_helper) = FileDaemon::start(database, user_pool.clone());

    let (web_server, server_handler) = router_start(bind, user_pool, file_event_helper.clone());

    let file_watcher = FileWatcher::start(".", config_path, file_event_helper.clone());

    //let web_server = WebServer::router_start(bind, user_pool.clone());

    wait_to_stop(async || {
        server_handler.shutdown();
        file_event_helper
            .send_terminate()
            .await
            .tap_none(|| warn!("Unable send event to file daemon, maybe consumer has dropped!"));
    })
    .await;

    web_server.await??;

    file_watcher.stop(|| warn!("File watcher thread not stopped"));

    file_daemon.into_inner().await??;

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
                .default_value(DEFAULT_WAIT_TIME_STR),
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
