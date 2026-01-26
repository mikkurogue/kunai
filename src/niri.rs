use std::process::Command;

use anyhow::{
    Result,
    anyhow,
};
use serde_json::Value;

/// Get available keyboard layouts from niri
pub fn get_layouts() -> Result<Vec<String>> {
    let output = Command::new("niri")
        .args(&["msg", "--json", "keyboard-layouts"])
        .output()?;

    let json: Value = serde_json::from_slice(&output.stdout)?;
    let names = json["names"]
        .as_array()
        .ok_or_else(|| anyhow!("Invalid niri response"))?;

    Ok(names
        .iter()
        .filter_map(|v| v.as_str().map(String::from))
        .collect())
}

/// Get current layout index
pub fn get_current_index() -> Result<u32> {
    let output = Command::new("niri")
        .args(&["msg", "--json", "keyboard-layouts"])
        .output()?;

    let json: Value = serde_json::from_slice(&output.stdout)?;
    Ok(json["current_idx"].as_u64().unwrap_or(0) as u32)
}

/// Switch to target layout (cycles with keyboard-layout-next)
pub fn switch_to_layout(target: u32) -> Result<()> {
    let current = get_current_index()?;
    if current == target {
        return Ok(());
    }

    let layouts = get_layouts()?;
    let total = layouts.len() as u32;

    // Calculate shortest path (forward wrapping)
    let steps = (target + total - current) % total;

    for _ in 0..steps {
        Command::new("niri")
            .args(&["msg", "action", "switch-layout", "next"])
            .output()?;
    }

    Ok(())
}
