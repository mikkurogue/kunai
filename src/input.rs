use std::{
    collections::{
        BTreeMap,
        HashSet,
    },
    fs,
    path::{
        Path,
        PathBuf,
    },
    sync::Arc,
};

use anyhow::Result;
use evdev::{
    BusType,
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

struct ProbeResult {
    name:      String,
    vendor_id: u16,
    product_id: u16,
    bus_type:  BusType,
}

fn probe_keyboard(path: &Path) -> Option<ProbeResult> {
    let device = Device::open(path).ok()?;
    let name = device.name().unwrap_or("Unknown");

    if name.contains("Receiver") {
        return None;
    }

    let has_key_a = device
        .supported_keys()
        .is_some_and(|keys| keys.contains(KeyCode::KEY_A));

    if !has_key_a {
        return None;
    }

    let id = device.input_id();
    Some(ProbeResult {
        name: name.to_string(),
        vendor_id: id.vendor(),
        product_id: id.product(),
        bus_type: id.bus_type(),
    })
}

pub fn list_keyboards() -> Result<Vec<Keyboard>> {
    let mut keyboards: BTreeMap<(u16, u16), Keyboard> = BTreeMap::new();

    // Scan /dev/input/by-id/ for USB keyboard interfaces.
    //
    // We only process *-event-kbd symlinks. This matches both primary
    // (-event-kbd) and secondary (-ifNN-event-kbd) interfaces, while
    // excluding non-keyboard HID interfaces (e.g. gaming mouse -keyboard).
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
        let filename = match path.file_name().and_then(|n| n.to_str()) {
            Some(f) => f,
            None => continue,
        };

        if !filename.ends_with("-event-kbd") {
            continue;
        }

        let probe = match probe_keyboard(&path) {
            Some(p) => p,
            None => continue,
        };

        let vid_pid = (probe.vendor_id, probe.product_id);
        let device_path = fs::canonicalize(&path)?;

        // Prefer primary interfaces over secondary; secondary interfaces
        // (-ifNN-) often claim KEY_A but don't actually produce events.
        let is_primary = !filename.contains("-if");

        if is_primary {
            keyboards.insert(
                vid_pid,
                Keyboard {
                    name:        probe.name,
                    device_path,
                    vendor_id:   probe.vendor_id,
                    product_id:  probe.product_id,
                },
            );
        } else {
            keyboards.entry(vid_pid).or_insert_with(|| Keyboard {
                name:        probe.name,
                device_path,
                vendor_id:   probe.vendor_id,
                product_id:  probe.product_id,
            });
        }
    }

    // Scan /dev/input/event* for Bluetooth and embedded keyboards.
    //
    // Only accept Bluetooth, i8042 (PS/2), and I2C buses. USB keyboards
    // are already covered by the by-id scan above, so we skip USB here
    // to avoid picking up gaming peripheral keyboard interfaces.
    for entry in fs::read_dir("/dev/input/")? {
        let entry = entry?;
        let path = entry.path();
        let filename = match path.file_name().and_then(|n| n.to_str()) {
            Some(f) => f,
            None => continue,
        };

        if !filename.starts_with("event") {
            continue;
        }

        let probe = match probe_keyboard(&path) {
            Some(p) => p,
            None => continue,
        };

        if !matches!(
            probe.bus_type,
            BusType::BUS_BLUETOOTH | BusType::BUS_I8042 | BusType::BUS_I2C
        ) {
            continue;
        }

        let vid_pid = (probe.vendor_id, probe.product_id);

        keyboards.entry(vid_pid).or_insert_with(|| Keyboard {
            name:        probe.name,
            device_path: fs::canonicalize(&path).unwrap_or(path),
            vendor_id:   probe.vendor_id,
            product_id:  probe.product_id,
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
