extern crate actix;
extern crate actix_web;
extern crate env_logger;
extern crate futures;
#[macro_use]
extern crate log;
extern crate mapbox_expressions_to_sql;
extern crate num_cpus;
extern crate r2d2;
extern crate r2d2_postgres;
extern crate serde;
#[macro_use]
extern crate serde_derive;
extern crate serde_json;
extern crate tilejson;

use actix::{Actor, Addr, SyncArbiter};
use actix_web::server;
use std::env;
use std::error::Error;
use std::io;

mod coordinator_actor;
mod db;
mod martin;
mod messages;
mod source;
mod utils;
mod worker_actor;

fn main() {
    env_logger::init();

    let conn_string: String = env::var("DATABASE_URL").expect("DATABASE_URL must be set");

    let pool_size = env::var("DATABASE_POOL_SIZE")
        .ok()
        .and_then(|pool_size| pool_size.parse::<u32>().ok())
        .unwrap_or(20);

    let worker_processes = env::var("WORKER_PROCESSES")
        .ok()
        .and_then(|worker_processes| worker_processes.parse::<usize>().ok())
        .unwrap_or(num_cpus::get());

    let keep_alive = env::var("KEEP_ALIVE")
        .ok()
        .and_then(|keep_alive| keep_alive.parse::<usize>().ok())
        .unwrap_or(75);

    info!("Connecting to {}", conn_string);
    let pool = match db::setup_connection_pool(&conn_string, pool_size) {
        Ok(pool) => {
            info!("Connected to postgres: {}", conn_string);
            pool
        }
        Err(error) => {
            error!("Can't connect to postgres: {}", error);
            std::process::exit(-1);
        }
    };

    let sources = match pool.get()
        .map_err(|err| io::Error::new(io::ErrorKind::Other, err.description()))
        .and_then(|conn| source::get_sources(conn))
    {
        Ok(sources) => sources,
        Err(error) => {
            error!("Can't load sources: {}", error);
            std::process::exit(-1);
        }
    };

    let server = actix::System::new("server");
    let coordinator_addr: Addr<_> = coordinator_actor::CoordinatorActor::default().start();
    let db_sync_arbiter = SyncArbiter::start(3, move || db::DbExecutor(pool.clone()));

    let port = 3000;
    let bind_addr = format!("0.0.0.0:{}", port);
    let _addr = server::new(move || {
        martin::new(
            db_sync_arbiter.clone(),
            coordinator_addr.clone(),
            sources.clone(),
        )
    }).bind(bind_addr.clone())
        .expect(&format!("Can't bind to {}", bind_addr))
        .keep_alive(keep_alive)
        .shutdown_timeout(0)
        .workers(worker_processes)
        .start();

    let _ = server.run();
    info!("Server has been started on {}.", bind_addr);
}
