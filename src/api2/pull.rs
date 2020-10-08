//! Sync datastore from remote server
use std::sync::{Arc};

use anyhow::{format_err, Error};
use futures::{select, future::FutureExt};

use proxmox::api::api;
use proxmox::api::{ApiMethod, Router, RpcEnvironment, Permission};

use crate::server::{WorkerTask, jobstate::Job};
use crate::backup::DataStore;
use crate::client::{HttpClient, HttpClientOptions, BackupRepository, pull::pull_store};
use crate::api2::types::*;
use crate::config::{
    remote,
    sync::SyncJobConfig,
    acl::{PRIV_DATASTORE_BACKUP, PRIV_DATASTORE_PRUNE, PRIV_REMOTE_READ},
    cached_user_info::CachedUserInfo,
};


pub fn check_pull_privs(
    auth_id: &Authid,
    store: &str,
    remote: &str,
    remote_store: &str,
    delete: bool,
) -> Result<(), Error> {

    let user_info = CachedUserInfo::new()?;

    user_info.check_privs(auth_id, &["datastore", store], PRIV_DATASTORE_BACKUP, false)?;
    user_info.check_privs(auth_id, &["remote", remote, remote_store], PRIV_REMOTE_READ, false)?;

    if delete {
        user_info.check_privs(auth_id, &["datastore", store], PRIV_DATASTORE_PRUNE, false)?;
    }

    Ok(())
}

pub async fn get_pull_parameters(
    store: &str,
    remote: &str,
    remote_store: &str,
) -> Result<(HttpClient, BackupRepository, Arc<DataStore>), Error> {

    let tgt_store = DataStore::lookup_datastore(store)?;

    let (remote_config, _digest) = remote::config()?;
    let remote: remote::Remote = remote_config.lookup("remote", remote)?;

    let options = HttpClientOptions::new()
        .password(Some(remote.password.clone()))
        .fingerprint(remote.fingerprint.clone());

    let src_repo = BackupRepository::new(Some(remote.userid.clone()), Some(remote.host.clone()), remote.port, remote_store.to_string());

    let client = HttpClient::new(&src_repo.host(), src_repo.port(), &src_repo.auth_id(), options)?;
    let _auth_info = client.login() // make sure we can auth
        .await
        .map_err(|err| format_err!("remote connection to '{}' failed - {}", remote.host, err))?;


    Ok((client, src_repo, tgt_store))
}

pub fn do_sync_job(
    mut job: Job,
    sync_job: SyncJobConfig,
    auth_id: &Authid,
    schedule: Option<String>,
) -> Result<String, Error> {

    let job_id = job.jobname().to_string();
    let worker_type = job.jobtype().to_string();

    let email = crate::server::lookup_user_email(auth_id.user());

    let upid_str = WorkerTask::spawn(
        &worker_type,
        Some(job.jobname().to_string()),
        auth_id.clone(),
        false,
        move |worker| async move {

            job.start(&worker.upid().to_string())?;

            let worker2 = worker.clone();
            let sync_job2 = sync_job.clone();

            let worker_future = async move {

                let delete = sync_job.remove_vanished.unwrap_or(true);
                let (client, src_repo, tgt_store) = get_pull_parameters(&sync_job.store, &sync_job.remote, &sync_job.remote_store).await?;

                worker.log(format!("Starting datastore sync job '{}'", job_id));
                if let Some(event_str) = schedule {
                    worker.log(format!("task triggered by schedule '{}'", event_str));
                }
                worker.log(format!("Sync datastore '{}' from '{}/{}'",
                        sync_job.store, sync_job.remote, sync_job.remote_store));

                let backup_auth_id = Authid::backup_auth_id();

                crate::client::pull::pull_store(&worker, &client, &src_repo, tgt_store.clone(), delete, backup_auth_id.clone()).await?;

                worker.log(format!("sync job '{}' end", &job_id));

                Ok(())
            };

            let mut abort_future = worker2.abort_future().map(|_| Err(format_err!("sync aborted")));

            let result = select!{
                worker = worker_future.fuse() => worker,
                abort = abort_future => abort,
            };

            let status = worker2.create_state(&result);

            match job.finish(status) {
                Ok(_) => {},
                Err(err) => {
                    eprintln!("could not finish job state: {}", err);
                }
            }

            if let Some(email) = email {
                if let Err(err) = crate::server::send_sync_status(&email, &sync_job2, &result) {
                    eprintln!("send sync notification failed: {}", err);
                }
            }

            result
        })?;

    Ok(upid_str)
}

#[api(
    input: {
        properties: {
            store: {
                schema: DATASTORE_SCHEMA,
            },
            remote: {
                schema: REMOTE_ID_SCHEMA,
            },
            "remote-store": {
                schema: DATASTORE_SCHEMA,
            },
            "remove-vanished": {
                schema: REMOVE_VANISHED_BACKUPS_SCHEMA,
                optional: true,
            },
        },
    },
    access: {
        // Note: used parameters are no uri parameters, so we need to test inside function body
        description: r###"The user needs Datastore.Backup privilege on '/datastore/{store}',
and needs to own the backup group. Remote.Read is required on '/remote/{remote}/{remote-store}'.
The delete flag additionally requires the Datastore.Prune privilege on '/datastore/{store}'.
"###,
        permission: &Permission::Anybody,
    },
)]
/// Sync store from other repository
async fn pull (
    store: String,
    remote: String,
    remote_store: String,
    remove_vanished: Option<bool>,
    _info: &ApiMethod,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<String, Error> {

    let auth_id: Authid = rpcenv.get_auth_id().unwrap().parse()?;
    let delete = remove_vanished.unwrap_or(true);

    check_pull_privs(&auth_id, &store, &remote, &remote_store, delete)?;

    let (client, src_repo, tgt_store) = get_pull_parameters(&store, &remote, &remote_store).await?;

    // fixme: set to_stdout to false?
    let upid_str = WorkerTask::spawn("sync", Some(store.clone()), auth_id.clone(), true, move |worker| async move {

        worker.log(format!("sync datastore '{}' start", store));

        let pull_future = pull_store(&worker, &client, &src_repo, tgt_store.clone(), delete, auth_id);
        let future = select!{
            success = pull_future.fuse() => success,
            abort = worker.abort_future().map(|_| Err(format_err!("pull aborted"))) => abort,
        };

        let _ = future?;

        worker.log(format!("sync datastore '{}' end", store));

        Ok(())
    })?;

    Ok(upid_str)
}

pub const ROUTER: Router = Router::new()
    .post(&API_METHOD_PULL);
