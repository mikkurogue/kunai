use std::{
    collections::HashMap,
    fs,
    path::PathBuf,
};

use anyhow::Result;
use evdev::Device;

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
