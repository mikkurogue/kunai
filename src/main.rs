mod config;
mod input;
mod niri;

use std::{
    collections::{
        HashMap,
        HashSet,
    },
    sync::Arc,
    time::{
        Duration,
        Instant,
    },
};

use anyhow::Result;
use clap::{
    Parser,
    Subcommand,
};
use config::{
    Config,
    KeyboardConfig,
};
use evdev::Device;
use rusb::{
    Context,
    HotplugBuilder,
    UsbContext,
};
use tokio::{
    sync::mpsc,
    task::JoinHandle,
};

use crate::input::HotPlugHandler;

#[derive(Parser)]
#[command(name = "kunai")]
#[command(about = "Per-keyboard layout switcher for Niri", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// List detected keyboards
    List,

    /// Interactive setup to map keyboards to layouts
    Setup,

    /// Run as background daemon
    Daemon {
        /// Dry-run mode: print layout switches without actually switching
        #[arg(long)]
        dry_run: bool,
    },

    /// Test mode: show which keyboard generates events
    Test,
}

struct MonitoredKeyboard {
    name:        String,
    task_handle: JoinHandle<()>,
}

struct DaemonState {
    layout_map:          HashMap<String, (String, u32)>, // "vid:pid" -> (name, layout_idx)
    monitored_keyboards: HashMap<String, MonitoredKeyboard>, // "vid:pid" -> monitor info
}

fn main() -> Result<()> {
    // Initialize tracing early
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive(tracing::Level::INFO.into()),
        )
        .init();

    let cli = Cli::parse();

    match cli.command {
        Commands::List => cmd_list(),
        Commands::Setup => cmd_setup(),
        Commands::Daemon { dry_run } => {
            let runtime = tokio::runtime::Runtime::new()?;
            runtime.block_on(cmd_daemon(dry_run))
        }
        Commands::Test => {
            let runtime = tokio::runtime::Runtime::new()?;
            runtime.block_on(cmd_test())
        }
    }
}

fn cmd_list() -> Result<()> {
    let keyboards = input::list_keyboards()?;

    if keyboards.is_empty() {
        println!("No keyboards found.");
        println!("\nNote: You may need to be in the 'input' group:");
        println!("  sudo usermod -aG input $USER");
        println!("  (then log out and back in)");
        return Ok(());
    }

    println!("Found {} keyboard(s):\n", keyboards.len());
    for (i, kb) in keyboards.iter().enumerate() {
        println!("{}. {}", i + 1, kb.name);
        println!("   Path: {:?}", kb.device_path);
        println!("   ID: {:04x}:{:04x}\n", kb.vendor_id, kb.product_id);
    }

    Ok(())
}

fn cmd_setup() -> Result<()> {
    let keyboards = input::list_keyboards()?;
    let layouts = niri::get_layouts()?;

    if keyboards.is_empty() {
        anyhow::bail!("No keyboards detected. Check permissions.");
    }

    println!("Found {} keyboard(s)", keyboards.len());
    println!("\nAvailable niri layouts:");
    for (i, layout) in layouts.iter().enumerate() {
        println!("  [{}] {}", i, layout);
    }

    let mut config = Config { keyboards: vec![] };

    for kb in keyboards {
        println!("\nConfigure: {}", kb.name);
        let index: usize = dialoguer::Input::new()
            .with_prompt(format!("Layout index (0-{})", layouts.len() - 1))
            .validate_with(|input: &usize| {
                if *input < layouts.len() {
                    Ok(())
                } else {
                    Err(format!("Invalid index. Must be 0-{}", layouts.len() - 1))
                }
            })
            .interact()?;

        config.keyboards.push(KeyboardConfig {
            name:         kb.name,
            vendor_id:    format!("{:04x}", kb.vendor_id),
            product_id:   format!("{:04x}", kb.product_id),
            layout_index: index as u32,
        });
    }

    config.save()?;

    println!("\n✓ Configuration saved!");
    println!("\nAdd to your niri config (~/.config/niri/config.kdl):");
    println!("  spawn-at-startup \"kunai\" \"daemon\"");
    println!("\nOr run manually:");
    println!("  kunai daemon");

    Ok(())
}

fn write_error_dump(error: &anyhow::Error) -> Result<()> {
    use std::io::Write;

    let dump_path = dirs::config_dir()
        .ok_or_else(|| anyhow::anyhow!("Could not find config directory"))?
        .join("kunai")
        .join("dump.txt");

    let timestamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&dump_path)?;

    writeln!(file, "\n========== ERROR DUMP {} ==========", timestamp)?;
    writeln!(file, "Error: {:#}", error)?;
    writeln!(file, "\nBacktrace:")?;
    writeln!(file, "{:?}", error)?;
    writeln!(file, "=====================================\n")?;

    tracing::error!("Error details written to: {}", dump_path.display());

    Ok(())
}

fn run_hotplug_monitor(
    configured_devices: Arc<HashSet<(u16, u16)>>,
    signal_tx: std::sync::mpsc::Sender<()>,
) -> Result<()> {
    let context = Context::new()?;

    let _reg: rusb::Registration<Context> = HotplugBuilder::new()
        .enumerate(false) // Don't enumerate on registration
        .register(
            &context,
            Box::new(HotPlugHandler {
                configured_devices,
                signal_tx,
            }),
        )?;

    tracing::info!("USB hotplug monitoring started");

    loop {
        if let Err(e) = context.handle_events(None) {
            tracing::error!("USB context error: {}", e);
            return Err(e.into());
        }
    }
}

async fn manage_keyboard_monitors(
    state: &mut DaemonState,
    event_tx: mpsc::UnboundedSender<(String, u32)>,
) -> Result<()> {
    tracing::info!("Re-enumerating keyboards...");

    let current_keyboards = input::list_keyboards().map_err(|e| {
        tracing::error!("Failed to enumerate keyboards: {}", e);
        e
    })?;

    let mut current_device_ids: HashSet<String> = HashSet::new();

    // Start monitoring new keyboards
    for kb in current_keyboards {
        let device_id = format!("{:04x}:{:04x}", kb.vendor_id, kb.product_id);
        current_device_ids.insert(device_id.clone());

        // Skip if already monitoring
        if state.monitored_keyboards.contains_key(&device_id) {
            continue;
        }

        // Check if device is in config
        if let Some((name, layout_idx)) = state.layout_map.get(&device_id).cloned() {
            let device = Device::open(&kb.device_path)?;
            let stream = device.into_event_stream()?;
            let tx = event_tx.clone();
            let device_id_clone = device_id.clone();
            let name_clone = name.clone();

            let handle = tokio::spawn(async move {
                tracing::info!("Started monitoring: {} → layout {}", name_clone, layout_idx);

                monitor_keyboard(device_id_clone.clone(), layout_idx, stream, tx).await;

                tracing::info!("Stopped monitoring: {} ({})", name_clone, device_id_clone);
            });

            state.monitored_keyboards.insert(
                device_id.clone(),
                MonitoredKeyboard {
                    name:        name.clone(),
                    task_handle: handle,
                },
            );

            tracing::info!(
                "Now monitoring: {} ({}) → layout {}",
                name,
                device_id,
                layout_idx
            );
        }
    }

    // Remove disconnected keyboards
    let disconnected: Vec<String> = state
        .monitored_keyboards
        .keys()
        .filter(|id| !current_device_ids.contains(*id))
        .cloned()
        .collect();

    for device_id in disconnected {
        if let Some(monitor) = state.monitored_keyboards.remove(&device_id) {
            monitor.task_handle.abort(); // Cancel the monitoring task
            tracing::info!("Stopped monitoring: {} ({})", monitor.name, device_id);
        }
    }

    tracing::info!("Active monitors: {}", state.monitored_keyboards.len());

    Ok(())
}

async fn cmd_daemon(dry_run: bool) -> Result<()> {
    let config = Config::load()?;

    if config.keyboards.is_empty() {
        anyhow::bail!("No keyboards configured. Run 'kunai setup' first.");
    }

    // Build configured device set for hotplug filtering
    let configured_devices: HashSet<(u16, u16)> = config
        .keyboards
        .iter()
        .filter_map(|kb| {
            let vid = u16::from_str_radix(&kb.vendor_id, 16).ok()?;
            let pid = u16::from_str_radix(&kb.product_id, 16).ok()?;
            Some((vid, pid))
        })
        .collect();

    // Build layout map
    let mut layout_map = HashMap::new();
    for kb in &config.keyboards {
        layout_map.insert(
            format!("{}:{}", kb.vendor_id, kb.product_id),
            (kb.name.clone(), kb.layout_index),
        );
    }

    // Channel for keyboard events (async)
    let (event_tx, mut event_rx) = mpsc::unbounded_channel();

    // Channel for hotplug signals (sync → async bridge)
    let (hotplug_tx, hotplug_rx) = std::sync::mpsc::channel();

    // Start USB hotplug monitoring thread
    if rusb::has_hotplug() {
        tracing::info!("Starting USB hotplug monitoring");
        let configured = Arc::new(configured_devices);
        std::thread::spawn(move || {
            if let Err(e) = run_hotplug_monitor(configured, hotplug_tx) {
                tracing::error!("Hotplug monitor failed: {}", e);
            }
        });
    } else {
        tracing::warn!("USB hotplug not supported on this system");
    }

    // Initialize daemon state
    let mut state = DaemonState {
        layout_map,
        monitored_keyboards: HashMap::new(),
    };

    // Initial device enumeration
    tracing::info!("Performing initial keyboard enumeration");
    manage_keyboard_monitors(&mut state, event_tx.clone()).await?;

    if state.monitored_keyboards.is_empty() {
        anyhow::bail!("No configured keyboards found");
    }

    if dry_run {
        tracing::info!("DRY-RUN MODE: Layout switches will be printed but not executed");
    } else {
        tracing::info!("Daemon started. Waiting for keyboard events...");
    }

    let mut last_device = String::new();
    let mut last_switch = Instant::now();

    // Wrap hotplug receiver in Arc<Mutex> for shared access
    let hotplug_rx = Arc::new(std::sync::Mutex::new(hotplug_rx));

    // Main event loop
    loop {
        tokio::select! {
            // Keyboard event received
            Some((device_id, target_layout)) = event_rx.recv() => {
                // Debounce: only switch if different device
                if device_id != last_device && last_switch.elapsed() > Duration::from_millis(100) {
                    if dry_run {
                        tracing::info!(
                            "[DRY-RUN] Would switch to layout {} for device {}",
                            target_layout, device_id
                        );
                    } else {
                        if let Err(e) = niri::switch_to_layout(target_layout) {
                            tracing::error!("Failed to switch layout: {}", e);
                        } else {
                            tracing::debug!("Switched to layout {} for device {}", target_layout, device_id);
                            last_device = device_id;
                            last_switch = Instant::now();
                        }
                    }
                }
            }

            // USB device change detected
            result = tokio::task::spawn_blocking({
                let rx = Arc::clone(&hotplug_rx);
                move || {
                    rx.lock().unwrap().recv()
                }
            }) => {
                if let Ok(Ok(_)) = result {
                    tracing::info!("USB device change detected");
                    if let Err(e) = manage_keyboard_monitors(&mut state, event_tx.clone()).await {
                        tracing::error!("Failed to re-enumerate devices: {}", e);
                        if let Err(dump_err) = write_error_dump(&e) {
                            tracing::error!("Failed to write error dump: {}", dump_err);
                        }
                        return Err(e);
                    }
                }
            }
        }
    }
}

async fn monitor_keyboard(
    device_id: String,
    target_layout: u32,
    mut stream: evdev::EventStream,
    tx: mpsc::UnboundedSender<(String, u32)>,
) {
    loop {
        match stream.next_event().await {
            Ok(event) if event.value() == 1 => {
                // Key press detected, send to main loop
                tracing::trace!("Key press from device {}", device_id);
                let _ = tx.send((device_id.clone(), target_layout));
            }
            Ok(_) => {} // Ignore key releases
            Err(e) => {
                // Device disconnected or error
                tracing::info!("Device {} stream ended: {}", device_id, e);
                break;
            }
        }
    }
}

async fn cmd_test() -> Result<()> {
    let keyboards = input::list_keyboards()?;

    if keyboards.is_empty() {
        anyhow::bail!("No keyboards detected. Check permissions.");
    }

    println!("Monitoring keyboards... (press Ctrl+C to stop)\n");

    let (tx, mut rx) = mpsc::unbounded_channel();

    // Spawn a task for each keyboard
    for kb in keyboards {
        let name = kb.name.clone();
        let device = Device::open(&kb.device_path)?;
        let stream = device.into_event_stream()?;
        let tx = tx.clone();

        tokio::spawn(async move {
            test_keyboard(name, stream, tx).await;
        });
    }

    drop(tx);

    // Print events as they come
    while let Some(name) = rx.recv().await {
        let now = chrono::Local::now();
        println!("[{}] Event from: {}", now.format("%H:%M:%S"), name);
    }

    Ok(())
}

async fn test_keyboard(
    name: String,
    mut stream: evdev::EventStream,
    tx: mpsc::UnboundedSender<String>,
) {
    loop {
        match stream.next_event().await {
            Ok(event) if event.value() == 1 => {
                let _ = tx.send(name.clone());
            }
            Ok(_) => {}
            Err(_) => break,
        }
    }
}
