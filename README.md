# keebect

Per-keyboard layout switcher for Niri compositor. Automatically switches keyboard layouts based on which physical keyboard you're typing on.

## What it does

keebect monitors your keyboards and switches to a configured layout when you start typing on a specific keyboard. Perfect for multi-keyboard setups where different keyboards use different layouts (e.g., QWERTY laptop keyboard + Colemak mechanical keyboard).

## Requirements

- Linux with Niri compositor
- Rust toolchain (cargo)
- Access to `/dev/input/event*` devices

## Installation

```bash
./install.sh
```

This will:
- Build the release binary
- Install to `/usr/local/bin/keebect`
- Set up udev rules for keyboard access (no reboot/logout required)

## Setup

1. List available keyboards:
```bash
keebect list
```

2. Run interactive setup to map keyboards to layouts:
```bash
keebect setup
```

3. Test the configuration (optional):
```bash
keebect daemon --dry-run
```

4. Add to Niri startup config (`~/.config/niri/config.kdl`):
```kdl
spawn-at-startup { command ["keebect" "daemon"]; }
```

Or run manually:
```bash
keebect daemon
```

## Commands

- `keebect list` - List detected keyboards with IDs
- `keebect setup` - Interactive configuration for keyboard-to-layout mapping
- `keebect daemon` - Run as background daemon
- `keebect daemon --dry-run` - Test mode, prints switches without applying
- `keebect test` - Show which keyboard generates events

## Configuration

Configuration is stored in `~/.config/keebect/config.toml` and maps keyboard vendor/product IDs to Niri layout indices.
