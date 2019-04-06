extern crate proxmox_backup;

//use proxmox_backup::tools;
use proxmox_backup::api_schema::router::*;
use proxmox_backup::api_schema::config::*;
use proxmox_backup::server::rest::*;
use proxmox_backup::server;
use proxmox_backup::tools::daemon;
use proxmox_backup::auth_helpers::*;
use proxmox_backup::config;

use failure::*;
use lazy_static::lazy_static;

use futures::future::Future;

use hyper;

fn main() {

    if let Err(err) = run() {
        eprintln!("Error: {}", err);
        std::process::exit(-1);
    }
}

fn run() -> Result<(), Error> {
    if let Err(err) = syslog::init(
        syslog::Facility::LOG_DAEMON,
        log::LevelFilter::Info,
        Some("proxmox-backup-api")) {
        bail!("unable to inititialize syslog - {}", err);
    }

    server::create_task_log_dir()?;

    config::create_configdir()?;

    if let Err(err) = generate_auth_key() {
        bail!("unable to generate auth key - {}", err);
    }
    let _ = private_auth_key(); // load with lazy_static

    if let Err(err) = generate_csrf_key() {
        bail!("unable to generate csrf key - {}", err);
    }
    let _ = csrf_secret(); // load with lazy_static

    lazy_static!{
       static ref ROUTER: Router = proxmox_backup::api2::router();
    }

    let config = ApiConfig::new(
        env!("PROXMOX_JSDIR"), &ROUTER, RpcEnvironmentType::PRIVILEGED);

    let rest_server = RestServer::new(config);

    // http server future:
    let server = daemon::create_daemon(
        ([127,0,0,1], 82).into(),
        |listener| {
            Ok(hyper::Server::builder(listener.incoming())
                .serve(rest_server)
                .map_err(|e| eprintln!("server error: {}", e))
            )
        },
    )?;

    hyper::rt::run(server);

    Ok(())
}
