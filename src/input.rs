use std::{
    collections::{
        BTreeMap,
        HashSet,
    },
    fs,
    path::PathBuf,
    sync::Arc,
};

use anyhow::Result;
use evdev::{
    Device,
    KeyCode,
};
use rusb::{
    Hotplug,
    UsbContext,
};

pub struct Keyboard {
    pub name:        String,
    pub device_path: PathBuf,
    pub vendor_id:   u16,
    pub product_id:  u16,
}

/// List physical keyboards by scanning input devices.
///
/// Scans both `/dev/input/by-id/` (stable symlinks for USB devices) and
/// `/dev/input/event*` directly (to catch Bluetooth HID and other keyboards
/// not represented in by-id). Deduplicates by (vendor_id, product_id).
pub fn list_keyboards() -> Result<Vec<Keyboard>> {
    // Dedup key: (vendor_id, product_id) — matches the config/monitoring key scheme.
    // Using an ordered map for deterministic output order.
    let mut keyboards: BTreeMap<(u16, u16), Keyboard> = BTreeMap::new();

    // -- Scan 1: /dev/input/by-id/ (stable USB symlinks) --
    let by_id_path = "/dev/input/by-id/";
    if !PathBuf::from(by_id_path).exists() {
        anyhow::bail!(
            "Cannot access {}. Are you in the 'input' group?",
            by_id_path
        );
    }

    for entry in fs::read_dir(by_id_path)? {
        let entry = entry?;
        let path = entry.path();

        let device = match Device::open(&path) {
            Ok(d) => d,
            Err(_) => continue,
        };

        let name = device.name().unwrap_or("Unknown");

        let is_keyboard = device
            .supported_keys()
            .is_some_and(|keys| keys.contains(KeyCode::KEY_A));

        if !is_keyboard || name.contains("Receiver") {
            continue;
        }

        let input_id = device.input_id();
        let vid_pid = (input_id.vendor(), input_id.product());

        keyboards.insert(
            vid_pid,
            Keyboard {
                name:        name.to_string(),
                device_path: fs::canonicalize(&path)?,
                vendor_id:   input_id.vendor(),
                product_id:  input_id.product(),
            },
        );
    }

    // -- Scan 2: /dev/input/event* directly (Bluetooth HID, etc.) --
    let input_path = "/dev/input/";
    for entry in fs::read_dir(input_path)? {
        let entry = entry?;
        let path = entry.path();
        let filename = path.file_name().and_then(|n| n.to_str()).unwrap_or("");

        if !filename.starts_with("event") {
            continue;
        }

        let device = match Device::open(&path) {
            Ok(d) => d,
            Err(_) => continue,
        };

        let name = device.name().unwrap_or("Unknown");

        let is_keyboard = device
            .supported_keys()
            .is_some_and(|keys| keys.contains(KeyCode::KEY_A));

        if !is_keyboard || name.contains("Receiver") {
            continue;
        }

        let input_id = device.input_id();
        let vid_pid = (input_id.vendor(), input_id.product());

        // Only insert if this VID:PID hasn't been seen yet (by-id entries take priority)
        keyboards.entry(vid_pid).or_insert_with(|| Keyboard {
            name:        name.to_string(),
            device_path: fs::canonicalize(&path).unwrap_or(path),
            vendor_id:   input_id.vendor(),
            product_id:  input_id.product(),
        });
    }

    Ok(keyboards.into_values().collect())
}

pub struct HotPlugHandler {
    pub configured_devices: Arc<HashSet<(u16, u16)>>,
    pub signal_tx:          std::sync::mpsc::Sender<()>,
}

impl<T: UsbContext> Hotplug<T> for HotPlugHandler {
    fn device_arrived(&mut self, device: rusb::Device<T>) {
        let device_desc = match device.device_descriptor() {
            Ok(desc) => desc,
            Err(_) => return,
        };

        let vid = device_desc.vendor_id();
        let pid = device_desc.product_id();

        // Only signal if this device is in config
        if self.configured_devices.contains(&(vid, pid)) {
            tracing::info!("Configured keyboard detected: {:04x}:{:04x}", vid, pid);
            let _ = self.signal_tx.send(());
        } else {
            tracing::debug!("Ignoring non-configured device: {:04x}:{:04x}", vid, pid);
        }
    }

    fn device_left(&mut self, device: rusb::Device<T>) {
        let device_desc = match device.device_descriptor() {
            Ok(desc) => desc,
            Err(_) => return,
        };

        let vid = device_desc.vendor_id();
        let pid = device_desc.product_id();

        // Only signal if this device is in config
        if self.configured_devices.contains(&(vid, pid)) {
            tracing::info!("Configured keyboard disconnected: {:04x}:{:04x}", vid, pid);
            let _ = self.signal_tx.send(());
        } else {
            tracing::debug!(
                "Ignoring non-configured device removal: {:04x}:{:04x}",
                vid,
                pid
            );
        }
    }
}
