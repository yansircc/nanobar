# nanobar

A lightweight macOS menu bar manager. Hide menu bar icons without needing a full app like Bartender or Ice.

nanobar works by creating an invisible `NSStatusItem` divider in your menu bar. When activated, it expands to 10000pt, pushing everything to its left off-screen. This is the same technique used by Bartender and Ice — no private APIs, no SIP disabling, no accessibility permissions.

## Install

```bash
cargo install --path .
```

Requires macOS and Rust toolchain.

## Usage

```
nanobar <command>
```

### Commands

| Command | Description |
|---------|-------------|
| `list` | List all menu bar items with their positions |
| `start` | Start the daemon (adds a `›` divider to the menu bar) |
| `hide [apps...]` | Hide items to the left of the divider |
| `show` | Show all hidden items |
| `stop` | Stop the daemon and remove the divider |
| `status` | Show current daemon state and item visibility |
| `install` | Install launch agent for auto-start at login |
| `uninstall` | Remove launch agent |

### Examples

Start the daemon — a `›` divider appears in your menu bar. Drag it to choose where to split visible/hidden items:

```bash
nanobar start
```

Hide everything to the left of the divider:

```bash
nanobar hide
```

Hide specific apps (moves the divider automatically):

```bash
nanobar hide Spotlight "Creative Cloud"
```

You can also reference items by their number from `nanobar list`:

```bash
nanobar list
#    App                     PID   Window       X     W
  1  Spotlight              1234    12345     200    22
  2  Creative Cloud         5678    67890     222    30  <-- divider
  ...

nanobar hide 1 2
```

Show all hidden items:

```bash
nanobar show
```

Stop the daemon and remove the divider:

```bash
nanobar stop
```

### Auto-start

To have nanobar start automatically at login:

```bash
nanobar install
```

This creates a LaunchAgent at `~/Library/LaunchAgents/nanobar.plist`. To remove it:

```bash
nanobar uninstall
```

You can also toggle this from the right-click menu (see below).

### Click to Toggle

The divider works as a toggle button:

- `›` (visible state) — **click to hide** items to its left
- `‹` (hidden state) — **click to show** all items

### Right-click Menu

Right-click the divider to open a context menu:

- **Start at Login** — toggle auto-start (shows ✓ when enabled)
- **Quit** — stop the daemon

## How It Works

1. `nanobar start` spawns a background daemon that creates an `NSStatusItem` divider (`›`) and an invisible pusher item
2. The divider's position is persisted via `NSStatusItem.autosaveName`, so it survives restarts
3. `nanobar hide` (or clicking `›`) tells the daemon to expand the pusher to 10000pt, pushing left-side items off-screen, and changes the divider to `‹`
4. `nanobar show` (or clicking `‹`) collapses the pusher back and restores the divider to `›`
5. `nanobar hide <apps>` reads each app's saved `NSStatusItem Preferred Position` from `defaults` and repositions the divider accordingly

The daemon communicates with the CLI over a Unix socket at `/tmp/nanobar.sock`. AppKit calls are dispatched to the main thread via `dispatch_async_f`.

## Requirements

- macOS (tested on macOS 15 Sequoia)
- Screen Recording permission (for `nanobar list` to read window info via `CGWindowListCopyWindowInfo`)

## License

MIT
