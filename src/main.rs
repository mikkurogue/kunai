mod config;
mod input;
mod niri;

use std::collections::HashMap;
use std::time::{Duration, Instant};

use anyhow::Result;
use clap::{Parser, Subcommand};
use config::{Config, KeyboardConfig};
use evdev::Device;
use tokio::sync::mpsc;

#[derive(Parser)]
#[command(name = "keebect")]
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

fn main() -> Result<()> {
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
            name: kb.name,
            vendor_id: format!("{:04x}", kb.vendor_id),
            product_id: format!("{:04x}", kb.product_id),
            layout_index: index as u32,
        });
    }

    config.save()?;

    println!("\n✓ Configuration saved!");
    println!("\nAdd to your niri config (~/.config/niri/config.kdl):");
    println!("  spawn-at-startup {{ command [\"keebect\" \"daemon\"]; }}");
    println!("\nOr run manually:");
    println!("  keebect daemon");

    Ok(())
}

async fn cmd_daemon(dry_run: bool) -> Result<()> {
    let config = Config::load()?;

    if config.keyboards.is_empty() {
        anyhow::bail!("No keyboards configured. Run 'keebect setup' first.");
    }

    // Build device ID -> (name, layout) map
    let mut layout_map = HashMap::new();
    for kb in &config.keyboards {
        layout_map.insert(
            format!("{}:{}", kb.vendor_id, kb.product_id),
            (kb.name.clone(), kb.layout_index),
        );
    }

    // Open configured keyboards
    let keyboards = input::list_keyboards()?;

    // Channel to receive keyboard events
    let (tx, mut rx) = mpsc::unbounded_channel();

    let mut monitored_count = 0;

    // Spawn a task for each configured keyboard
    for kb in keyboards {
        let key = format!("{:04x}:{:04x}", kb.vendor_id, kb.product_id);
        if let Some((name, layout_idx)) = layout_map.get(&key).cloned() {
            let device = Device::open(&kb.device_path)?;
            let stream = device.into_event_stream()?;
            let tx = tx.clone();

            tokio::spawn(async move {
                println!("Monitoring: {} → layout {}", name, layout_idx);
                monitor_keyboard(key, layout_idx, stream, tx).await;
            });

            monitored_count += 1;
        }
    }

    drop(tx); // Drop original sender

    if monitored_count == 0 {
        anyhow::bail!("No configured keyboards found");
    }

    if dry_run {
        println!("\n[DRY-RUN MODE] Layout switches will be printed but not executed\n");
    } else {
        println!("\nDaemon started. Waiting for keyboard events...\n");
    }

    let mut last_device = String::new();
    let mut last_switch = Instant::now();

    // Process events from any keyboard
    while let Some((device_id, target_layout)) = rx.recv().await {
        // Debounce: only switch if different device
        if device_id != last_device && last_switch.elapsed() > Duration::from_millis(100) {
            if dry_run {
                println!(
                    "[DRY-RUN] Would switch to layout {} for device {}",
                    target_layout, device_id
                );
            } else {
                if let Err(e) = niri::switch_to_layout(target_layout) {
                    eprintln!("Failed to switch layout: {}", e);
                } else {
                    last_device = device_id;
                    last_switch = Instant::now();
                }
            }
        }
    }

    Ok(())
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
                let _ = tx.send((device_id.clone(), target_layout));
            }
            Ok(_) => {} // Ignore key releases
            Err(_e) => {
                // Device disconnected or error
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
