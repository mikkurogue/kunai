use std::{
    collections::{
        HashMap,
        HashSet,
    },
    fs,
    path::PathBuf,
    sync::Arc,
};

use anyhow::Result;
use evdev::Device;
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

/// List physical keyboards (name contains "Keyboard", not "Receiver")
pub fn list_keyboards() -> Result<Vec<Keyboard>> {
    let mut keyboards = HashMap::new();

    // Read /dev/input/by-id/ for stable device paths
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
        let filename = path.file_name().and_then(|n| n.to_str()).unwrap_or("");

        // Only process *-event-kbd entries (primary keyboard interface)
        if !filename.ends_with("-event-kbd") {
            continue;
        }

        // Try to open the device
        let device = match Device::open(&path) {
            Ok(d) => d,
            Err(_) => continue, // Skip if we can't open (permissions)
        };

        let name = device.name().unwrap_or("Unknown");

        // Filter: must contain "Keyboard" and NOT contain "Receiver"
        // Should somehow come up with a more reliable way to identify keyboards
        // I'm not yet sure how to do this with evdev - multiple vendors use different naming
        // conventions or the system cant assign the proper device "type" to it
        if !name.contains("Keyboard") || name.contains("Receiver") {
            continue;
        }

        let input_id = device.input_id();
        let (vendor_id, product_id) = (input_id.vendor(), input_id.product());

        // Deduplicate by name (HashMap automatically handles this)
        keyboards.insert(
            name.to_string(),
            Keyboard {
                name: name.to_string(),
                device_path: fs::canonicalize(&path)?, // Resolve symlink to /dev/input/eventX
                vendor_id,
                product_id,
            },
        );
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
