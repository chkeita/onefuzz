use crate::tasks::config::CommonConfig;
use crate::tasks::utils::parse_key_value;
use anyhow::Result;
use clap::{App, Arg, ArgMatches};
use onefuzz::monitor::DirectoryMonitor;
use reqwest::Url;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};
use tokio::stream::StreamExt;

use tempfile::tempdir;
use uuid::Uuid;

pub const SETUP_DIR: &str = "setup_dir";
pub const INPUTS_DIR: &str = "inputs_dir";
pub const CRASHES_DIR: &str = "crashes_dir";
pub const TARGET_WORKERS: &str = "target_workers";
pub const REPORTS_DIR: &str = "reports_dir";
pub const NO_REPRO_DIR: &str = "no_repro_dir";
pub const TARGET_TIMEOUT: &str = "target_timeout";
pub const CHECK_RETRY_COUNT: &str = "check_retry_count";
pub const DISABLE_CHECK_QUEUE: &str = "disable_check_queue";
pub const UNIQUE_REPORTS_DIR: &str = "unique_reports_dir";
pub const COVERAGE_DIR: &str = "coverage_dir";
pub const READONLY_INPUTS: &str = "readonly_inputs_dir";
pub const CHECK_ASAN_LOG: &str = "check_asan_log";
pub const TOOLS_DIR: &str = "tools_dir";
pub const RENAME_OUTPUT: &str = "rename_output";
pub const CHECK_FUZZER_HELP: &str = "check_fuzzer_help";

pub const TARGET_EXE: &str = "target_exe";
pub const TARGET_ENV: &str = "target_env";
pub const TARGET_OPTIONS: &str = "target_options";
// pub const SUPERVISOR_EXE: &str = "supervisor_exe";
// pub const SUPERVISOR_ENV: &str = "supervisor_env";
// pub const SUPERVISOR_OPTIONS: &str = "supervisor_options";
pub const GENERATOR_EXE: &str = "generator_exe";
pub const GENERATOR_ENV: &str = "generator_env";
pub const GENERATOR_OPTIONS: &str = "generator_options";

pub const ANALYZER_EXE: &str = "analyzer_exe";
pub const ANALYZER_OPTIONS: &str = "analyzer_options";
pub const ANALYZER_ENV: &str = "analyzer_env";
pub const ANALYSIS_DIR: &str = "analysis_dir";

pub enum CmdType {
    Target,
    Generator,
    // Supervisor,
}

pub fn add_cmd_options(
    cmd_type: CmdType,
    exe: bool,
    arg: bool,
    env: bool,
    mut app: App<'static, 'static>,
) -> App<'static, 'static> {
    let (exe_name, env_name, arg_name) = match cmd_type {
        CmdType::Target => (TARGET_EXE, TARGET_ENV, TARGET_OPTIONS),
        // CmdType::Supervisor => (SUPERVISOR_EXE, SUPERVISOR_ENV, SUPERVISOR_OPTIONS),
        CmdType::Generator => (GENERATOR_EXE, GENERATOR_ENV, GENERATOR_OPTIONS),
    };

    if exe {
        app = app.arg(Arg::with_name(exe_name).takes_value(true).required(true));
    }
    if env {
        app = app.arg(
            Arg::with_name(env_name)
                .long(env_name)
                .takes_value(true)
                .multiple(true),
        )
    }
    if arg {
        app = app.arg(
            Arg::with_name(arg_name)
                .long(arg_name)
                .takes_value(true)
                .value_delimiter(" ")
                .help("Use a quoted string with space separation to denote multiple arguments"),
        )
    }
    app
}

pub fn get_hash_map(args: &clap::ArgMatches<'_>, name: &str) -> Result<HashMap<String, String>> {
    let mut env = HashMap::new();
    for opt in args.values_of_lossy(name).unwrap_or_default() {
        let (k, v) = parse_key_value(opt)?;
        env.insert(k, v);
    }
    Ok(env)
}

pub fn get_cmd_exe(cmd_type: CmdType, args: &clap::ArgMatches<'_>) -> Result<String> {
    let name = match cmd_type {
        CmdType::Target => TARGET_EXE,
        // CmdType::Supervisor => SUPERVISOR_EXE,
        CmdType::Generator => GENERATOR_EXE,
    };

    let exe = value_t!(args, name, String)?;
    Ok(exe)
}

pub fn get_cmd_arg(cmd_type: CmdType, args: &clap::ArgMatches<'_>) -> Vec<String> {
    let name = match cmd_type {
        CmdType::Target => TARGET_OPTIONS,
        // CmdType::Supervisor => SUPERVISOR_OPTIONS,
        CmdType::Generator => GENERATOR_OPTIONS,
    };

    args.values_of_lossy(name).unwrap_or_default()
}

pub fn get_cmd_env(
    cmd_type: CmdType,
    args: &clap::ArgMatches<'_>,
) -> Result<HashMap<String, String>> {
    let env_name = match cmd_type {
        CmdType::Target => TARGET_ENV,
        // CmdType::Supervisor => SUPERVISOR_ENV,
        CmdType::Generator => GENERATOR_ENV,
    };
    get_hash_map(args, env_name)
}

pub fn add_common_config(app: App<'static, 'static>) -> App<'static, 'static> {
    app.arg(
        Arg::with_name("job_id")
            .long("job_id")
            .takes_value(true)
            .required(false),
    )
    .arg(
        Arg::with_name("task_id")
            .long("task_id")
            .takes_value(true)
            .required(false),
    )
    .arg(
        Arg::with_name("instance_id")
            .long("instance_id")
            .takes_value(true)
            .required(false),
    )
    .arg(
        Arg::with_name("setup_dir")
            .long("setup_dir")
            .takes_value(true)
            .required(false),
    )
}

fn get_uuid(name: &str, args: &ArgMatches<'_>) -> Result<Uuid> {
    match value_t!(args, name, String) {
        Ok(x) => Uuid::parse_str(&x)
            .map_err(|x| format_err!("invalid {}.  uuid expected.  {})", name, x)),
        Err(_) => Ok(Uuid::nil()),
    }
}

pub fn build_common_config(args: &ArgMatches<'_>) -> Result<CommonConfig> {
    let job_id = get_uuid("job_id", args)?;
    let task_id = get_uuid("task_id", args)?;
    let instance_id = get_uuid("instance_id", args)?;

    let setup_dir = if args.is_present(SETUP_DIR) {
        value_t!(args, SETUP_DIR, PathBuf)?
    } else if args.is_present(TARGET_EXE) {
        value_t!(args, TARGET_EXE, PathBuf)?
            .parent()
            .map(|x| x.to_path_buf())
            .unwrap_or_default()
    } else {
        PathBuf::default()
    };

    let config = CommonConfig {
        heartbeat_queue: None,
        instrumentation_key: None,
        telemetry_key: None,
        job_id,
        task_id,
        instance_id,
        setup_dir,
    };
    Ok(config)
}

/// Information about a local path being monitored
/// A new notification will be received on the queue url
/// For each new file added to the directory
pub struct DirectoryMonitorQueue {
    pub directory_path: PathBuf,
    pub queue_url: Url,
    pub handle: tokio::task::JoinHandle<std::result::Result<(), anyhow::Error>>,
}

impl DirectoryMonitorQueue {
    pub async fn start_monitoring(directory_path: impl AsRef<Path>) -> Result<Self> {
        let directory_path = PathBuf::from(directory_path.as_ref());
        let queue_file = tempdir()?;
        let queue_url =
            Url::from_file_path(queue_file.path()).map_err(|_| anyhow!("invalid file path"))?;

        let queue_url_clone = queue_url.clone();
        let directory_path_clone = directory_path.clone();
        let handle: tokio::task::JoinHandle<std::result::Result<(), anyhow::Error>> =
            tokio::spawn(async move {
                let queue = storage_queue::QueueClient::new(queue_url_clone.clone())?;

                let mut monitor = DirectoryMonitor::new(directory_path_clone.clone());
                monitor.start()?;
                while let Some(crash) = monitor.next().await {
                    let file_url =
                        Url::from_file_path(crash).map_err(|_| anyhow!("invalid file path"))?;
                    queue.enqueue(file_url).await?
                }
                Ok(())
            });

        Ok(DirectoryMonitorQueue {
            directory_path,
            queue_url,
            handle,
        })
    }
}
