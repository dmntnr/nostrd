use std::fs;
use std::io::{Read, Write};
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};
use std::process;

use clap::{Parser, Subcommand};

use nostrd::config::Config;

const PID_FILE: &str = "nostrd.pid";

#[derive(Parser)]
#[command(
    name = "nostrd",
    version,
    about = "A Nostr relay server",
    disable_version_flag = true
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,

    /// Print version
    #[arg(short = 'v', long = "version", action = clap::ArgAction::Version)]
    _version: (),

    /// Path to config file
    #[arg(short = 'c', long, default_value = "nostrd.toml")]
    config: PathBuf,

    /// Data directory
    #[arg(short = 'd', long, global = true)]
    data_dir: Option<PathBuf>,

    /// Verbose output
    #[arg(short = 'V', long, action = clap::ArgAction::Count, global = true)]
    verbose: u8,

    /// Run in foreground (don't daemonize)
    #[arg(short = 'f', long)]
    foreground: bool,

    /// Log file path (default: data_dir/nostrd.log)
    #[arg(long)]
    log_file: Option<PathBuf>,
}

#[derive(Subcommand)]
enum Command {
    /// Start the relay server (default)
    Start,
    /// Stop a running relay server
    Stop,
    /// Restart the relay server
    Restart,
    /// Show relay statistics
    Stats,
}

fn load_config(path: &Path) -> Result<Config, Box<dyn std::error::Error>> {
    let config = if path.exists() {
        let content = fs::read_to_string(path)?;
        toml::from_str(&content).map_err(|e| {
            format!(
                "Failed to parse config file {}: {}",
                path.display(),
                e.message()
            )
        })?
    } else {
        Config::default()
    };
    if config.max_event_tags == 0 {
        return Err("max_event_tags must be greater than 0".into());
    }
    if config.max_content_length == 0 {
        return Err("max_content_length must be greater than 0".into());
    }
    if config.lmdb_map_size_gb == 0 {
        return Err("lmdb_map_size_gb must be greater than 0".into());
    }
    if config.broadcast_channel_size == 0 {
        return Err("broadcast_channel_size must be greater than 0".into());
    }
    if config.max_connections == 0 {
        return Err("max_connections must be greater than 0".into());
    }
    if config.max_ws_message_size == 0 {
        return Err("max_ws_message_size must be greater than 0".into());
    }
    Ok(config)
}

fn pid_path(data_dir: &Path) -> PathBuf {
    data_dir.join(PID_FILE)
}

fn write_pid(data_dir: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let pid = process::id();
    let path = pid_path(data_dir);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    // Use O_EXCL to prevent multiple instances
    let f = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path);
    match f {
        Ok(mut f) => {
            writeln!(f, "{}", pid)?;
            Ok(())
        }
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => match read_pid(data_dir) {
            Ok(existing_pid) => Err(format!(
                "Another nostrd instance is already running (PID {}). \
                         Use 'nostrd stop' to stop it, or remove {} manually.",
                existing_pid,
                path.display()
            )
            .into()),
            Err(_) => Err(format!(
                "Lock file {} exists but cannot read PID. \
                     Remove it manually if no instance is running.",
                path.display()
            )
            .into()),
        },
        Err(e) => Err(e.into()),
    }
}

fn read_pid(data_dir: &Path) -> Result<u32, Box<dyn std::error::Error>> {
    let path = pid_path(data_dir);
    let mut f = fs::File::open(&path)?;
    let mut s = String::new();
    f.read_to_string(&mut s)?;
    Ok(s.trim().parse()?)
}

fn remove_pid(data_dir: &Path) {
    let _ = fs::remove_file(pid_path(data_dir));
}

fn cleanup_stale_pid(data_dir: &Path) {
    let path = pid_path(data_dir);
    if !path.exists() {
        return;
    }
    match read_pid(data_dir) {
        Ok(pid) => {
            let alive = unsafe { libc::kill(pid as i32, 0) };
            if alive != 0 {
                let _ = fs::remove_file(&path);
                tracing::info!("Removed stale PID file (process {} no longer running)", pid);
            }
        }
        Err(_) => {
            let _ = fs::remove_file(&path);
        }
    }
}

fn signal_process(pid: u32, sig: i32) -> Result<(), Box<dyn std::error::Error>> {
    unsafe {
        if libc::kill(pid as i32, sig) != 0 {
            let err = std::io::Error::last_os_error();
            return Err(format!("Failed to signal PID {}: {}", pid, err).into());
        }
    }
    Ok(())
}

fn cmd_stop(data_dir: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let pid = match read_pid(data_dir) {
        Ok(p) => p,
        Err(_) => {
            println!("No running nostrd instance found (no PID file)");
            return Ok(());
        }
    };
    println!("Stopping nostrd (PID {})...", pid);
    let signaled = signal_process(pid, libc::SIGTERM).is_ok();
    if signaled {
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
    remove_pid(data_dir);
    if !signaled {
        return Err(format!("Failed to signal PID {} (process may be already stopped)", pid).into());
    }
    println!("Stopped.");
    Ok(())
}

fn cmd_stats(data_dir: &Path) -> Result<(), Box<dyn std::error::Error>> {
    match nostrd::db::LmdbStore::open(data_dir) {
        Ok(store) => {
            let stats = store.stats()?;
            println!("nostrd relay statistics");
            println!("  Data directory: {}", data_dir.display());
            println!("  Stored events:  {}", stats.event_count);

            match read_pid(data_dir) {
                Ok(pid) => {
                    println!("  Running:        yes (PID {})", pid);
                }
                Err(_) => {
                    println!("  Running:        no");
                }
            }
        }
        Err(e) => {
            println!("nostrd relay statistics");
            println!("  Data directory: {}", data_dir.display());
            println!("  Status:         {}", e);
        }
    }
    Ok(())
}

fn daemonize(log_file: Option<&Path>) -> Result<(), Box<dyn std::error::Error>> {
    let pid = unsafe { libc::fork() };
    if pid < 0 {
        return Err("fork failed".into());
    }
    if pid > 0 {
        println!("Daemon started (PID: {})", pid);
        process::exit(0);
    }

    if unsafe { libc::setsid() } < 0 {
        return Err("setsid failed".into());
    }

    let devnull = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/null")?;
    unsafe {
        libc::dup2(devnull.as_raw_fd(), libc::STDIN_FILENO);
        libc::dup2(devnull.as_raw_fd(), libc::STDOUT_FILENO);
        if log_file.is_none() {
            libc::dup2(devnull.as_raw_fd(), libc::STDERR_FILENO);
        }
    }

    if let Some(path) = log_file {
        let log = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?;
        unsafe {
            libc::dup2(log.as_raw_fd(), libc::STDERR_FILENO);
        }
    }

    Ok(())
}

fn get_data_dir(cli: &Cli, _config: &Config) -> PathBuf {
    cli.data_dir.clone().unwrap_or_else(|| {
        PathBuf::from(std::env::var("NOSTRD_DATA_DIR").unwrap_or_else(|_| "./data".to_string()))
    })
}

fn daemon_log_path(data_dir: &Path) -> PathBuf {
    data_dir.join("nostrd.log")
}

fn app_main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();
    let config = load_config(&cli.config)?;
    let data_dir = get_data_dir(&cli, &config);

    match cli.command.as_ref().unwrap_or(&Command::Start) {
        Command::Start => {
            // Daemonize first (before any logging/tokio setup)
            if !cli.foreground {
                let log_file = cli.log_file.clone().unwrap_or(daemon_log_path(&data_dir));
                daemonize(Some(&log_file))?;
            }

            // Initialize tracing AFTER daemonization
            let log_level = match cli.verbose {
                0 => "info",
                1 => "debug",
                _ => "trace",
            };
            let log_file = cli.log_file.clone().unwrap_or(daemon_log_path(&data_dir));
            let log_file_f = fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&log_file)?;

            if cli.foreground {
                tracing_subscriber::fmt()
                    .with_env_filter(
                        tracing_subscriber::EnvFilter::try_from_default_env()
                            .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(log_level)),
                    )
                    .init();
            } else {
                tracing_subscriber::fmt()
                    .with_writer(std::sync::Mutex::new(log_file_f))
                    .with_ansi(false)
                    .with_env_filter(
                        tracing_subscriber::EnvFilter::try_from_default_env()
                            .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(log_level)),
                    )
                    .init();
            }

            tracing::info!(
                "nostrd {} starting (data: {})",
                env!("CARGO_PKG_VERSION"),
                data_dir.display()
            );
            tracing::info!("Supported NIPs: 1, 9, 11, 12, 18, 19, 23, 25, 28, 40, 42, 45, 77");

            let store = nostrd::db::LmdbStore::open_with_map_size(
                &data_dir,
                config
                    .lmdb_map_size_gb
                    .saturating_mul(1024)
                    .saturating_mul(1024)
                    .saturating_mul(1024),
                config.max_query_candidates,
            )?;
            let stats = store.stats()?;
            tracing::info!(
                "Opened LMDB store at {:?} with {} events",
                data_dir,
                stats.event_count
            );
            tracing::info!("Starting relay at {}", config.listen_addr);

            cleanup_stale_pid(&data_dir);
            write_pid(&data_dir)?;

            let rt = tokio::runtime::Runtime::new()?;
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                rt.block_on(async {
                    tokio::select! {
                        r = nostrd::server::run(config, store) => r,
                        _ = tokio::signal::ctrl_c() => {
                            tracing::info!("Shutdown signal received");
                            Ok(())
                        }
                    }
                })
            }));

            match result {
                Ok(Ok(())) => {}
                Ok(Err(e)) => {
                    tracing::error!("Server error: {}", e);
                }
                Err(panic_info) => {
                    let msg = if let Some(s) = panic_info.downcast_ref::<String>() {
                        s.clone()
                    } else if let Some(s) = panic_info.downcast_ref::<&str>() {
                        s.to_string()
                    } else {
                        "unknown panic".to_string()
                    };
                    tracing::error!("Server panicked: {}. Attempting graceful shutdown.", msg);
                }
            }

            remove_pid(&data_dir);
        }
        Command::Stop => {
            cmd_stop(&data_dir)?;
        }
        Command::Restart => {
            cmd_stop(&data_dir)?;
            let exe = std::env::current_exe()?;
            let mut cmd = process::Command::new(exe);
            cmd.arg("--config")
                .arg(cli.config.to_string_lossy().to_string());
            if let Some(ref dd) = cli.data_dir {
                cmd.arg("--data-dir").arg(dd);
            }
            if cli.foreground {
                cmd.arg("--foreground");
            }
            for _ in 0..cli.verbose {
                cmd.arg("--verbose");
            }
            if let Some(ref lf) = cli.log_file {
                cmd.arg("--log-file").arg(lf);
            }
            cmd.arg("start");
            let status = cmd.spawn()?;
            println!("Started new process (PID {})", status.id());
        }
        Command::Stats => {
            cmd_stats(&data_dir)?;
        }
    }

    Ok(())
}

fn main() {
    std::panic::set_hook(Box::new(|info| {
        let payload = if let Some(s) = info.payload().downcast_ref::<&str>() {
            s.to_string()
        } else if let Some(s) = info.payload().downcast_ref::<String>() {
            s.clone()
        } else {
            "unknown panic payload".to_string()
        };
        let location = info
            .location()
            .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
            .unwrap_or_else(|| "unknown location".to_string());
        let backtrace = std::backtrace::Backtrace::force_capture();
        eprintln!(
            "FATAL PANIC at {}\n  payload: {}\n{}",
            location, payload, backtrace
        );
        if let Ok(mut log_file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open("nostrd.panic.log")
        {
            use std::io::Write;
            let _ = writeln!(
                log_file,
                "PANIC at {}: {}\n{}",
                location, payload, backtrace
            );
        }
    }));

    if let Err(e) = app_main() {
        eprintln!("Error: {}", e);
        process::exit(1);
    }
}
