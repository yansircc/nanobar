# Nanobar

100 lines of Rust. A minimal macOS menu bar manager.

No Bartender, no Ice, no accessibility permissions, no Screen Recording permissions.

## How it works

Creates an invisible `NSStatusItem` pusher. On click, it expands to 10000pt, pushing icons to its left off-screen. Same native technique as Bartender/Ice — no private APIs, no SIP.

## Install

```bash
cargo install nanobar
```

## Usage

```bash
nanobar
```

Runs in the background automatically. A `›` separator appears in the menu bar.

- **⌘+Drag** `›` to adjust the separator position
- **Left-click** `›` to hide icons to its left (becomes `‹`), click again to restore
- **Right-click** → Quit

## Auto-start at login

```bash
cat > ~/Library/LaunchAgents/nanobar.plist << EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>Label</key><string>nanobar</string>
<key>ProgramArguments</key><array><string>$(which nanobar)</string></array>
<key>RunAtLoad</key><true/>
</dict></plist>
EOF
```

## Remove auto-start

```bash
rm ~/Library/LaunchAgents/nanobar.plist
```

## License

MIT
