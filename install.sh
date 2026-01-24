#!/bin/bash
set -e

echo "Installing keebect..."

# Build release binary
cargo build --release

# Install binary
sudo cp target/release/keebect /usr/local/bin/
echo "✓ Binary installed to /usr/local/bin/keebect"

# Install udev rule for keyboard access
echo "Setting up keyboard access permissions..."
cat > /tmp/99-keebect.rules << 'EOF'
# Allow users to access keyboard input devices for keebect
KERNEL=="event*", SUBSYSTEM=="input", ENV{ID_INPUT_KEYBOARD}=="1", TAG+="uaccess"
EOF

sudo cp /tmp/99-keebect.rules /etc/udev/rules.d/
echo "✓ Udev rule installed"

# Reload udev rules and trigger for existing devices
sudo udevadm control --reload-rules
sudo udevadm trigger --subsystem-match=input
echo "✓ Udev rules reloaded (no logout required)"

# Wait a moment for udev to apply
sleep 1

echo ""
echo "Installation complete!"
echo ""
echo "Next steps:"
echo "  1. Run: keebect list"
echo "  2. Run: keebect setup"
echo "  3. Run: keebect daemon --dry-run"
echo "  4. Add to niri config:"
echo "     spawn-at-startup { command [\"keebect\" \"daemon\"]; }"
