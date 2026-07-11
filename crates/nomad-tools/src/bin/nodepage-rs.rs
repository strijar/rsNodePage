use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, RwLock};

use clap::Parser;
use tokio::sync::mpsc;

use nomad_core::announce::{self, APP_NAME};
use nomad_core::{NomadConfig, handler, pages::PageIndex};
use nomad_tools::cli::Args;

use rns_identity::destination::Destination;
use rns_identity::identity::Identity;
use rns_runtime::link_manager::LinkManager;
use rns_transport::messages::TransportMessage;

pub fn resolve_config_dirs(config: Option<&str>, rnsconfig: Option<&str>) -> (PathBuf, PathBuf) {
    let config_dir = match config {
        Some(dir) => PathBuf::from(dir),
        None => default_nodepage_config_dir(),
    };
    let rns_config_dir = match rnsconfig {
        Some(dir) => rns_runtime::platform::resolve_config_dir(Some(dir)),
        None => rns_runtime::platform::resolve_config_dir(None),
    };
    (config_dir, rns_config_dir)
}

fn default_nodepage_config_dir() -> PathBuf {
    if cfg!(target_os = "windows") {
        return std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .map(|path| path.join("rsNodePage"))
            .unwrap_or_else(|| PathBuf::from(".rsNodePage"));
    }

    if cfg!(target_os = "android") {
        return PathBuf::from("/data/local/tmp/.rsNodePage");
    }

    let etc = PathBuf::from("/etc/rsNodePage");
    if etc.join("config").is_file() {
        return etc;
    }

    if let Ok(home) = std::env::var("HOME") {
        let xdg = PathBuf::from(&home).join(".config/rsNodePage");
        if xdg.join("config").is_file() {
            return xdg;
        }
        PathBuf::from(home).join(".rsNodePage")
    } else {
        PathBuf::from(".rsNodePage")
    }
}

#[tokio::main]
async fn main() {
    let args = Args::parse();

    if args.exampleconfig {
        print!("{}", NomadConfig::example());
        return;
    }

    let verbosity = match args.verbose {
        0 => tracing::Level::INFO,
        1 => tracing::Level::DEBUG,
        _ => tracing::Level::TRACE,
    };
    tracing_subscriber::fmt().with_max_level(verbosity).init();

    let (config_dir, rns_config_dir) =
        resolve_config_dirs(args.config.as_deref(), args.rnsconfig.as_deref());

    let config_path = config_dir.join("config");

    if !config_path.exists() {
        tracing::warn!(
            path = %config_path.display(),
            "no config file found, writing defaults (edit and restart to customize)"
        );
        if let Some(parent) = config_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(&config_path, NomadConfig::example());
    }

    let mut config = NomadConfig::load(&config_path);
    if let Some(pages) = args.pages {
        config.pages_dir = pages;
    }
    if let Some(files) = args.files {
        config.files_dir = files;
    }

    if !config.pages_dir.is_absolute() {
        config.pages_dir = Path::new(&config_dir).join(config.pages_dir);
    }

    if !config.files_dir.is_absolute() {
        config.files_dir = Path::new(&config_dir).join(config.files_dir);
    }

    std::fs::create_dir_all(&config.pages_dir).expect("could not create pages_dir");
    std::fs::create_dir_all(&config.files_dir).expect("could not create files_dir");

    tracing::info!(
        display_name = %config.display_name,
        pages_dir = %config.pages_dir.display(),
        files_dir = %config.files_dir.display(),
        "nodepage-rs starting"
    );

    // --- Bring up the Reticulum runtime (interfaces, transport actor) ---
    let shutdown = rns_runtime::lifecycle::ShutdownSignal::new();
    let shutdown_for_signal = shutdown.clone();
    tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            tracing::info!("shutting down");
            shutdown_for_signal.trigger();
        }
    });

    let rns_config_dir_str = rns_config_dir.to_string_lossy().to_string();
    let is_foreground = Arc::new(AtomicBool::new(true));
    let rns_handle = rns_runtime::reticulum::init(
        Some(&rns_config_dir_str),
        None,
        shutdown.clone(),
        is_foreground,
    )
    .await
    .expect("failed to initialize Reticulum runtime");
    let transport_tx = rns_handle.transport_tx.clone();

    // --- Load (or create) this node's identity ---
    let identity = load_or_create_identity(&config.identity_path);
    let destination_hash = Destination::hash_from_name_and_identity(APP_NAME, Some(&identity.hash));
    tracing::info!(
        hash = %hex::encode(destination_hash),
        "node destination: {APP_NAME}"
    );

    // --- Build the initial page/file index and register the destination ---
    let index = Arc::new(RwLock::new(PageIndex::scan(
        &config.pages_dir,
        &config.files_dir,
    )));

    let (delivery_tx, delivery_rx) =
        mpsc::channel::<rns_transport::link_messages::DestinationEvent>(64);
    let _ = transport_tx.try_send(TransportMessage::RegisterDestination {
        hash: destination_hash,
        app_name: APP_NAME.to_string(),
        delivery_tx: Some(delivery_tx),
    });

    let signing_key = identity.get_signing_key();
    let mut link_mgr = LinkManager::with_destination(
        transport_tx.clone(),
        delivery_rx,
        &identity,
        APP_NAME,
        signing_key,
    );

    let link_identities = link_mgr.link_identities_handle();
    link_mgr.set_request_handler_ex(handler::build_request_handler(
        index.clone(),
        config.allowed_identities.clone(),
        link_identities,
    ));

    tokio::spawn(async move {
        link_mgr.run().await;
    });

    // --- Periodic rescan so new/edited pages show up without a restart ---
    {
        let pages_dir = config.pages_dir.clone();
        let files_dir = config.files_dir.clone();
        let index = index.clone();
        let interval_secs = config.rescan_interval_seconds;
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(std::time::Duration::from_secs(interval_secs));
            loop {
                ticker.tick().await;
                let fresh = PageIndex::scan(&pages_dir, &files_dir);
                if let Ok(mut guard) = index.write() {
                    tracing::debug!(pages = fresh.len(), "nomad: rescanned");
                    *guard = fresh;
                }
            }
        });
    }

    // --- Announces ---
    if config.announce_at_start {
        announce::send_announce(
            &transport_tx,
            &identity,
            destination_hash,
            &config.display_name,
        );
    }
    {
        let transport_tx = transport_tx.clone();
        let identity_for_announce = identity.clone();
        let display_name = config.display_name.clone();
        let interval_mins = config.announce_interval_minutes;
        tokio::spawn(async move {
            let mut ticker =
                tokio::time::interval(std::time::Duration::from_secs(interval_mins * 60));
            ticker.tick().await; // first tick fires immediately; skip it, we already announced above
            loop {
                ticker.tick().await;
                announce::send_announce(
                    &transport_tx,
                    &identity_for_announce,
                    destination_hash,
                    &display_name,
                );
            }
        });
    }

    tracing::info!(
        "ready — serving {} known path(s)",
        index.read().map(|g| g.len()).unwrap_or(0)
    );

    // Block until Ctrl-C (the signal handler above triggers `shutdown`).
    shutdown.wait().await;
    tracing::info!("stopped");
}

fn load_or_create_identity(path: &Path) -> Identity {
    match Identity::from_file(path) {
        Ok(id) => id,
        Err(_) => {
            tracing::info!(path = %path.display(), "no identity found, creating a new one");
            let id = Identity::new();
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            id.to_file(path).expect("failed to persist identity");
            id
        }
    }
}
