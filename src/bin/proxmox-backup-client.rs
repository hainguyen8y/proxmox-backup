extern crate proxmox_backup;

use failure::*;
//use std::os::unix::io::AsRawFd;
use chrono::{DateTime, Local, TimeZone};
use std::path::{Path, PathBuf};
use std::collections::HashMap;

use proxmox_backup::tools;
use proxmox_backup::cli::*;
use proxmox_backup::api_schema::*;
use proxmox_backup::api_schema::router::*;
use proxmox_backup::client::*;
use proxmox_backup::backup::*;
//use proxmox_backup::backup::image_index::*;
//use proxmox_backup::config::datastore;
//use proxmox_backup::catar::encoder::*;
//use proxmox_backup::backup::datastore::*;

use serde_json::{json, Value};
use hyper::Body;
use std::sync::Arc;
use regex::Regex;

use lazy_static::lazy_static;

lazy_static! {
    static ref BACKUPSPEC_REGEX: Regex = Regex::new(r"^([a-zA-Z0-9_-]+\.(?:catar|raw)):(.+)$").unwrap();
}

fn backup_directory<P: AsRef<Path>>(
    client: &mut HttpClient,
    repo: &BackupRepository,
    dir_path: P,
    archive_name: &str,
    backup_id: &str,
    backup_time: DateTime<Local>,
    chunk_size: Option<u64>,
    all_file_systems: bool,
    verbose: bool,
) -> Result<(), Error> {

    let mut param = json!({
        "archive-name": archive_name,
        "backup-type": "host",
        "backup-id": backup_id,
        "backup-time": backup_time.timestamp(),
    });

    if let Some(size) = chunk_size {
        param["chunk-size"] = size.into();
    }

    let query = tools::json_object_to_query(param)?;

    let path = format!("api2/json/admin/datastore/{}/catar?{}", repo.store, query);

    let stream = CaTarBackupStream::open(dir_path.as_ref(), all_file_systems, verbose)?;

    let body = Body::wrap_stream(stream);

    client.upload("application/x-proxmox-backup-catar", body, &path)?;

    Ok(())
}

/****
fn backup_image(datastore: &DataStore, file: &std::fs::File, size: usize, target: &str, chunk_size: usize) -> Result<(), Error> {

    let mut target = PathBuf::from(target);

    if let Some(ext) = target.extension() {
        if ext != "fidx" {
            bail!("got wrong file extension - expected '.fidx'");
        }
    } else {
        target.set_extension("fidx");
    }

    let mut index = datastore.create_image_writer(&target, size, chunk_size)?;

    tools::file_chunker(file, chunk_size, |pos, chunk| {
        index.add_chunk(pos, chunk)?;
        Ok(true)
    })?;

    index.close()?; // commit changes

    Ok(())
}
*/

fn strip_chunked_file_expenstions(list: Vec<String>) -> Vec<String> {

    let mut result = vec![];

    for file in list.into_iter() {
        if file.ends_with(".didx") {
            result.push(file[..file.len()-5].to_owned());
        } else if file.ends_with(".fidx") {
            result.push(file[..file.len()-5].to_owned());
        } else {
            result.push(file); // should not happen
        }
    }

    result
}

fn list_backups(
    param: Value,
    _info: &ApiMethod,
    _rpcenv: &mut RpcEnvironment,
) -> Result<Value, Error> {

    let repo_url = tools::required_string_param(&param, "repository")?;
    let repo = BackupRepository::parse(repo_url)?;

    let mut client = HttpClient::new(&repo.host, &repo.user);

    let path = format!("api2/json/admin/datastore/{}/backups", repo.store);

    let result = client.get(&path)?;

    // fixme: implement and use output formatter instead ..
    let list = result["data"].as_array().unwrap();

    for item in list {

        let id = item["backup-id"].as_str().unwrap();
        let btype = item["backup-type"].as_str().unwrap();
        let epoch = item["backup-time"].as_i64().unwrap();

        let backup_dir = BackupDir::new(btype, id, epoch);

        let files = item["files"].as_array().unwrap().iter().map(|v| v.as_str().unwrap().to_owned()).collect();
        let files = strip_chunked_file_expenstions(files);

        for filename in files {
            let path = backup_dir.relative_path().to_str().unwrap().to_owned();
            println!("{} | {}/{}", backup_dir.backup_time().format("%c"), path, filename);
        }
    }

    //Ok(result)
    Ok(Value::Null)
}

fn list_backup_groups(
    param: Value,
    _info: &ApiMethod,
    _rpcenv: &mut RpcEnvironment,
) -> Result<Value, Error> {

    let repo_url = tools::required_string_param(&param, "repository")?;
    let repo = BackupRepository::parse(repo_url)?;

    let mut client = HttpClient::new(&repo.host, &repo.user);

    let path = format!("api2/json/admin/datastore/{}/groups", repo.store);

    let mut result = client.get(&path)?;

    // fixme: implement and use output formatter instead ..
    let list = result["data"].as_array_mut().unwrap();

    list.sort_unstable_by(|a, b| {
        let a_id = a["backup-id"].as_str().unwrap();
        let a_backup_type = a["backup-type"].as_str().unwrap();
        let b_id = b["backup-id"].as_str().unwrap();
        let b_backup_type = b["backup-type"].as_str().unwrap();

        let type_order = a_backup_type.cmp(b_backup_type);
        if type_order == std::cmp::Ordering::Equal {
            a_id.cmp(b_id)
        } else {
            type_order
        }
    });

    for item in list {

        let id = item["backup-id"].as_str().unwrap();
        let btype = item["backup-type"].as_str().unwrap();
        let epoch = item["last-backup"].as_i64().unwrap();
        let last_backup = Local.timestamp(epoch, 0);
        let backup_count = item["backup-count"].as_u64().unwrap();

        let group = BackupGroup::new(btype, id);

        let path = group.group_path().to_str().unwrap().to_owned();

        let files = item["files"].as_array().unwrap().iter().map(|v| v.as_str().unwrap().to_owned()).collect();
        let files = strip_chunked_file_expenstions(files);

        println!("{:20} | {} | {:5} | {}", path, last_backup.format("%c"),
                 backup_count, tools::join(&files, ' '));
    }

    //Ok(result)
    Ok(Value::Null)
}

fn list_snapshots(
    param: Value,
    _info: &ApiMethod,
    _rpcenv: &mut RpcEnvironment,
) -> Result<Value, Error> {

    let repo_url = tools::required_string_param(&param, "repository")?;
    let repo = BackupRepository::parse(repo_url)?;

    let path = tools::required_string_param(&param, "group")?;
    let group = BackupGroup::parse(path)?;

    let query = tools::json_object_to_query(json!({
        "backup-type": group.backup_type(),
        "backup-id": group.backup_id(),
    }))?;

    let mut client = HttpClient::new(&repo.host, &repo.user);

    let path = format!("api2/json/admin/datastore/{}/snapshots?{}", repo.store, query);

    // fixme: params
    let result = client.get(&path)?;

    // fixme: implement and use output formatter instead ..
    let list = result["data"].as_array().unwrap();

    for item in list {

        let id = item["backup-id"].as_str().unwrap();
        let btype = item["backup-type"].as_str().unwrap();
        let epoch = item["backup-time"].as_i64().unwrap();

        let snapshot = BackupDir::new(btype, id, epoch);

        let path = snapshot.relative_path().to_str().unwrap().to_owned();

        let files = item["files"].as_array().unwrap().iter().map(|v| v.as_str().unwrap().to_owned()).collect();
        let files = strip_chunked_file_expenstions(files);

        println!("{} | {} | {}", path, snapshot.backup_time().format("%c"), tools::join(&files, ' '));
    }

    Ok(Value::Null)
}

fn forget_snapshots(
    param: Value,
    _info: &ApiMethod,
    _rpcenv: &mut RpcEnvironment,
) -> Result<Value, Error> {

    let repo_url = tools::required_string_param(&param, "repository")?;
    let repo = BackupRepository::parse(repo_url)?;

    let path = tools::required_string_param(&param, "snapshot")?;
    let snapshot = BackupDir::parse(path)?;

    let query = tools::json_object_to_query(json!({
        "backup-type": snapshot.group().backup_type(),
        "backup-id": snapshot.group().backup_id(),
        "backup-time": snapshot.backup_time().timestamp(),
    }))?;

    let mut client = HttpClient::new(&repo.host, &repo.user);

    let path = format!("api2/json/admin/datastore/{}/snapshots?{}", repo.store, query);

    let result = client.delete(&path)?;

    Ok(result)
}

fn start_garbage_collection(
    param: Value,
    _info: &ApiMethod,
    _rpcenv: &mut RpcEnvironment,
) -> Result<Value, Error> {

    let repo_url = tools::required_string_param(&param, "repository")?;
    let repo = BackupRepository::parse(repo_url)?;

    let mut client = HttpClient::new(&repo.host, &repo.user);

    let path = format!("api2/json/admin/datastore/{}/gc", repo.store);

    let result = client.post(&path)?;

    Ok(result)
}

fn parse_backupspec(value: &str) -> Result<(&str, &str), Error> {

    if let Some(caps) = BACKUPSPEC_REGEX.captures(value) {
        return Ok((caps.get(1).unwrap().as_str(), caps.get(2).unwrap().as_str()));
    }
    bail!("unable to parse directory specification '{}'", value);
}

fn create_backup(
    param: Value,
    _info: &ApiMethod,
    _rpcenv: &mut RpcEnvironment,
) -> Result<Value, Error> {

    let repo_url = tools::required_string_param(&param, "repository")?;

    let backupspec_list = tools::required_array_param(&param, "backupspec")?;

    let repo = BackupRepository::parse(repo_url)?;

    let all_file_systems = param["all-file-systems"].as_bool().unwrap_or(false);

    let verbose = param["verbose"].as_bool().unwrap_or(false);

    let chunk_size_opt = param["chunk-size"].as_u64().map(|v| v*1024);

    if let Some(size) = chunk_size_opt {
        verify_chunk_size(size)?;
    }

    let backup_id = param["host-id"].as_str().unwrap_or(&tools::nodename());

    let mut upload_list = vec![];

    for backupspec in backupspec_list {
        let (target, filename) = parse_backupspec(backupspec.as_str().unwrap())?;

        let stat = match nix::sys::stat::stat(filename) {
            Ok(s) => s,
            Err(err) => bail!("unable to access '{}' - {}", filename, err),
        };

        if (stat.st_mode & libc::S_IFDIR) != 0 {

            upload_list.push((filename.to_owned(), target.to_owned()));

        } else if (stat.st_mode & (libc::S_IFREG|libc::S_IFBLK)) != 0 {
            if stat.st_size <= 0 { bail!("got strange file size '{}'", stat.st_size); }
            let _size = stat.st_size as usize;

            panic!("implement me");

            //backup_image(&datastore, &file, size, &target, chunk_size)?;

            // let idx = datastore.open_image_reader(target)?;
            // idx.print_info();

        } else {
            bail!("unsupported file type (expected a directory, file or block device)");
        }
    }

    let backup_time = Local.timestamp(Local::now().timestamp(), 0);

    let mut client = HttpClient::new(&repo.host, &repo.user);

    client.login()?; // login before starting backup

    println!("Starting backup");
    println!("Client name: {}", tools::nodename());
    println!("Start Time: {}", backup_time.to_rfc3339());

    for (filename, target) in upload_list {
        println!("Upload '{}' to '{:?}' as {}", filename, repo, target);
        backup_directory(&mut client, &repo, &filename, &target, backup_id, backup_time,
                         chunk_size_opt, all_file_systems, verbose)?;
    }

    let end_time = Local.timestamp(Local::now().timestamp(), 0);
    let elapsed = end_time.signed_duration_since(backup_time);
    println!("Duration: {}", elapsed);

    println!("End Time: {}", end_time.to_rfc3339());

    Ok(Value::Null)
}

pub fn complete_backup_source(arg: &str, param: &HashMap<String, String>) -> Vec<String> {

    let mut result = vec![];

    let data: Vec<&str> = arg.splitn(2, ':').collect();

    if data.len() != 2 {
        result.push(String::from("root.catar:/"));
        result.push(String::from("etc.catar:/etc"));
        return result;
    }

    let files = tools::complete_file_name(data[1], param);

    for file in files {
        result.push(format!("{}:{}", data[0], file));
    }

    result
}

fn restore(
    param: Value,
    _info: &ApiMethod,
    _rpcenv: &mut RpcEnvironment,
) -> Result<Value, Error> {

    let repo_url = tools::required_string_param(&param, "repository")?;
    let repo = BackupRepository::parse(repo_url)?;

    let archive_name = tools::required_string_param(&param, "archive-name")?;

    let mut client = HttpClient::new(&repo.host, &repo.user);

    let path = tools::required_string_param(&param, "snapshot")?;

    let query;

    if path.matches('/').count() == 1 {
        let group = BackupGroup::parse(path)?;

        let subquery = tools::json_object_to_query(json!({
            "backup-type": group.backup_type(),
            "backup-id": group.backup_id(),
        }))?;

        let path = format!("api2/json/admin/datastore/{}/snapshots?{}", repo.store, subquery);
        let result = client.get(&path)?;

        let list = result["data"].as_array().unwrap();
        if list.len() == 0 {
            bail!("backup group '{}' does not contain any snapshots:", path);
        }

        query = tools::json_object_to_query(json!({
            "backup-type": group.backup_type(),
            "backup-id": group.backup_id(),
            "backup-time": list[0]["backup-time"].as_i64().unwrap(),
            "archive-name": archive_name,
        }))?;
    } else {
        let snapshot = BackupDir::parse(path)?;

        query = tools::json_object_to_query(json!({
            "backup-type": snapshot.group().backup_type(),
            "backup-id": snapshot.group().backup_id(),
            "backup-time": snapshot.backup_time().timestamp(),
            "archive-name": archive_name,
        }))?;
    }

    let target = tools::required_string_param(&param, "target")?;

    if archive_name.ends_with(".catar") {
        let path = format!("api2/json/admin/datastore/{}/catar?{}", repo.store, query);

        println!("DOWNLOAD FILE {} to {}", path, target);

        let target = PathBuf::from(target);
        let writer = CaTarBackupWriter::new(&target, true)?;
        client.download(&path, Box::new(writer))?;
    } else {
        bail!("unknown file extensions - unable to download '{}'", archive_name);
    }

    Ok(Value::Null)
}

fn prune(
    mut param: Value,
    _info: &ApiMethod,
    _rpcenv: &mut RpcEnvironment,
) -> Result<Value, Error> {

    let repo_url = tools::required_string_param(&param, "repository")?;
    let repo = BackupRepository::parse(repo_url)?;

    let mut client = HttpClient::new(&repo.host, &repo.user);

    let path = format!("api2/json/admin/datastore/{}/prune", repo.store);

    param.as_object_mut().unwrap().remove("repository");

    let result = client.post_json(&path, param)?;

    Ok(result)
}

fn main() {

    let repo_url_schema: Arc<Schema> = Arc::new(
        StringSchema::new("Repository URL.")
            .format(BACKUP_REPO_URL.clone())
            .max_length(256)
            .into()
    );

    let backup_source_schema: Arc<Schema> = Arc::new(
        StringSchema::new("Backup source specification ([<label>:<path>]).")
            .format(Arc::new(ApiStringFormat::Pattern(&BACKUPSPEC_REGEX)))
            .into()
    );

    let backup_cmd_def = CliCommand::new(
        ApiMethod::new(
            create_backup,
            ObjectSchema::new("Create (host) backup.")
                .required("repository", repo_url_schema.clone())
                .required(
                    "backupspec",
                    ArraySchema::new(
                        "List of backup source specifications ([<label.ext>:<path>] ...)",
                        backup_source_schema,
                    ).min_length(1)
                )
                .optional(
                    "verbose",
                    BooleanSchema::new("Verbose output.").default(false))
                .optional(
                    "host-id",
                    StringSchema::new("Use specified ID for the backup group name ('host/<id>'). The default is the system hostname."))
                .optional(
                    "chunk-size",
                    IntegerSchema::new("Chunk size in KB. Must be a power of 2.")
                        .minimum(64)
                        .maximum(4096)
                        .default(4096)
                )
        ))
        .arg_param(vec!["repository", "backupspec"])
        .completion_cb("backupspec", complete_backup_source);

    let list_cmd_def = CliCommand::new(
        ApiMethod::new(
            list_backup_groups,
            ObjectSchema::new("List backup groups.")
                .required("repository", repo_url_schema.clone())
        ))
        .arg_param(vec!["repository"]);

    let snapshots_cmd_def = CliCommand::new(
        ApiMethod::new(
            list_snapshots,
            ObjectSchema::new("List backup snapshots.")
                .required("repository", repo_url_schema.clone())
                .required("group", StringSchema::new("Backup group."))
        ))
        .arg_param(vec!["repository", "group"]);

    let forget_cmd_def = CliCommand::new(
        ApiMethod::new(
            forget_snapshots,
            ObjectSchema::new("Forget (remove) backup snapshots.")
                .required("repository", repo_url_schema.clone())
                .required("snapshot", StringSchema::new("Snapshot path."))
        ))
        .arg_param(vec!["repository", "snapshot"]);

    let garbage_collect_cmd_def = CliCommand::new(
        ApiMethod::new(
            start_garbage_collection,
            ObjectSchema::new("Start garbage collection for a specific repository.")
                .required("repository", repo_url_schema.clone())
        ))
        .arg_param(vec!["repository"]);

    let restore_cmd_def = CliCommand::new(
        ApiMethod::new(
            restore,
            ObjectSchema::new("Restore backup repository.")
                .required("repository", repo_url_schema.clone())
                .required("snapshot", StringSchema::new("Group/Snapshot path."))
                .required("archive-name", StringSchema::new("Backup archive name."))
                .required("target", StringSchema::new("Target directory path."))
        ))
        .arg_param(vec!["repository", "snapshot", "archive-name", "target"]);

    let prune_cmd_def = CliCommand::new(
        ApiMethod::new(
            prune,
            proxmox_backup::api2::admin::datastore::add_common_prune_prameters(
                ObjectSchema::new("Prune backup repository.")
                    .required("repository", repo_url_schema.clone())
            )
        ))
        .arg_param(vec!["repository"]);

    let cmd_def = CliCommandMap::new()
        .insert("backup".to_owned(), backup_cmd_def.into())
        .insert("forget".to_owned(), forget_cmd_def.into())
        .insert("garbage-collect".to_owned(), garbage_collect_cmd_def.into())
        .insert("list".to_owned(), list_cmd_def.into())
        .insert("prune".to_owned(), prune_cmd_def.into())
        .insert("restore".to_owned(), restore_cmd_def.into())
        .insert("snapshots".to_owned(), snapshots_cmd_def.into());

    run_cli_command(cmd_def.into());

}
