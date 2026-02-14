mod client;
mod daemon;
mod menubar;

use anyhow::{bail, Result};
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "nanobar", about = "macOS menu bar manager")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// List all menu bar items
    List,
    /// Start the nanobar daemon (adds a '|' divider to menu bar)
    Start,
    /// Hide menu bar items (optionally specify apps to set divider position)
    Hide {
        /// App names to hide (moves divider right of the rightmost specified app)
        apps: Vec<String>,
    },
    /// Show all hidden items
    Show,
    /// Stop the daemon and remove the divider
    Stop,
    /// Show current status
    Status,
    /// Internal: run as daemon process
    #[command(hide = true)]
    Daemon,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::List => {
            cmd_list();
            Ok(())
        }
        Commands::Start => cmd_start(),
        Commands::Hide { apps } => cmd_hide(&apps),
        Commands::Show => cmd_show(),
        Commands::Stop => cmd_stop(),
        Commands::Status => cmd_status(),
        Commands::Daemon => {
            daemon::run_daemon();
            Ok(())
        }
    }
}

fn cmd_list() {
    let items = menubar::list_menubar_items();
    let divider = items.iter().find(|i| i.owner_name == "nanobar");
    let expanded = divider.map(|d| d.width > 100.0).unwrap_or(false);

    println!(
        "{:>3}  {:<20} {:>6}  {:>7}  {:>6}  {:>4}",
        "#", "App", "PID", "Window", "X", "W"
    );
    for (i, item) in items.iter().enumerate() {
        let marker = if item.owner_name == "nanobar" {
            " <-- divider"
        } else if item.x < 0.0 {
            // Pushed off left edge of screen
            " [hidden]"
        } else if let Some(d) = divider {
            if !expanded && item.x < d.x {
                " [will hide]"
            } else {
                ""
            }
        } else {
            ""
        };
        println!(
            "{:>3}  {:<20} {:>6}  {:>7}  {:>6.0}  {:>4.0}{}",
            i + 1,
            item.owner_name,
            item.owner_pid,
            item.window_id,
            item.x,
            item.width,
            marker,
        );
    }
}

fn cmd_start() -> Result<()> {
    if client::is_daemon_running() {
        println!("daemon already running");
        return Ok(());
    }

    let exe = std::env::current_exe()?;
    std::process::Command::new(exe)
        .arg("daemon")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()?;

    // Wait for daemon to be ready
    for _ in 0..50 {
        std::thread::sleep(std::time::Duration::from_millis(100));
        if client::is_daemon_running() {
            println!("daemon started");
            return Ok(());
        }
    }

    bail!("daemon failed to start within 5 seconds");
}

fn cmd_hide(apps: &[String]) -> Result<()> {
    if !apps.is_empty() {
        // Move divider to hide specified apps, then expand
        move_divider_for_apps(apps)?;
    }

    // Ensure daemon is running
    if !client::is_daemon_running() {
        cmd_start()?;
    }

    let resp = client::send_command("hide")?;
    if resp == "ok" {
        if apps.is_empty() {
            println!("items left of divider hidden");
        }
    }
    Ok(())
}

/// Move divider to be just right of the rightmost specified app
fn move_divider_for_apps(apps: &[String]) -> Result<()> {
    let items = menubar::list_menubar_items();

    // Resolve numeric args (sequence numbers from `list`) to app names
    let resolved: Vec<String> = apps
        .iter()
        .map(|arg| {
            if let Ok(n) = arg.parse::<usize>() {
                if n >= 1 && n <= items.len() {
                    return items[n - 1].owner_name.clone();
                }
            }
            arg.clone()
        })
        .collect();

    // Find the rightmost target app (highest X = furthest right = should be just left of divider)
    let mut best_position: Option<f64> = None;
    let mut matched_names = Vec::new();

    for name in &resolved {
        let name_lower = name.to_lowercase();
        let matched: Vec<_> = items
            .iter()
            .filter(|item| {
                item.owner_name.to_lowercase().contains(&name_lower)
                    && item.owner_name != "nanobar"
            })
            .collect();

        if matched.is_empty() {
            eprintln!("  not found in menu bar: {}", name);
            continue;
        }

        for item in &matched {
            // Get bundle ID and preferred position
            if let Some(bundle_id) = menubar::get_bundle_id(item.owner_pid) {
                if let Some(pos) = menubar::get_preferred_position(&bundle_id) {
                    matched_names.push(item.owner_name.clone());
                    // We want the divider to have a LOWER position value than the target
                    // (lower value = further right in the menu bar)
                    // Take the minimum position among all targets
                    best_position = Some(match best_position {
                        Some(bp) => bp.min(pos),
                        None => pos,
                    });
                } else {
                    eprintln!(
                        "  no saved position for: {} ({})",
                        item.owner_name, bundle_id
                    );
                }
            } else {
                eprintln!("  cannot find bundle ID for: {}", item.owner_name);
            }
        }
    }

    let target_pos = match best_position {
        Some(p) => p,
        None => bail!("could not determine divider position for specified apps"),
    };

    // Find the rightmost matched item's X to estimate new divider placement
    let rightmost_target_x = matched_names.iter().filter_map(|name| {
        items.iter().find(|i| &i.owner_name == name).map(|i| i.x + i.width)
    }).max_by(|a, b| a.partial_cmp(b).unwrap());

    if let Some(cut_x) = rightmost_target_x {
        let also_hidden: Vec<_> = items
            .iter()
            .filter(|i| i.x < cut_x && i.owner_name != "nanobar" && !matched_names.contains(&i.owner_name))
            .collect();

        if also_hidden.is_empty() {
            println!("hiding: {}", matched_names.join(", "));
        } else {
            let also_names: Vec<_> = also_hidden.iter().map(|i| i.owner_name.as_str()).collect();
            println!("hiding: {} (also: {})", matched_names.join(", "), also_names.join(", "));
        }
    } else {
        println!("hiding: {}", matched_names.join(", "));
    }

    // Set nanobar position to slightly less than the target (further right)
    let new_pos = (target_pos - 20.0).max(1.0);

    // Stop daemon if running
    let was_running = client::is_daemon_running();
    if was_running {
        let _ = client::send_command("stop");
        std::thread::sleep(std::time::Duration::from_millis(300));
    }

    // Write new position
    let status = std::process::Command::new("defaults")
        .args([
            "write",
            "nanobar",
            "NSStatusItem Preferred Position Item-0",
            "-float",
            &format!("{:.1}", new_pos),
        ])
        .status()?;

    if !status.success() {
        bail!("failed to write position to defaults");
    }

    // Restart daemon
    cmd_start()?;

    Ok(())
}

fn cmd_show() -> Result<()> {
    let resp = client::send_command("show")?;
    if resp == "ok" {
        println!("all items visible");
    }
    Ok(())
}

fn cmd_stop() -> Result<()> {
    if !client::is_daemon_running() {
        println!("daemon not running");
        return Ok(());
    }
    let resp = client::send_command("stop")?;
    if resp == "ok" {
        println!("daemon stopped");
    }
    Ok(())
}

fn cmd_status() -> Result<()> {
    if !client::is_daemon_running() {
        println!("daemon: not running");
        println!("use 'nanobar start' to begin");
        return Ok(());
    }

    let state = client::send_command("state")?;
    println!("daemon: running");
    println!("state:  {}", state);

    // Show which items would be hidden
    let items = menubar::list_menubar_items();
    let divider = items.iter().find(|i| i.owner_name == "nanobar");
    if let Some(div) = divider {
        let hidden: Vec<_> = items
            .iter()
            .filter(|i| i.x < div.x && i.owner_name != "nanobar")
            .collect();
        let visible: Vec<_> = items
            .iter()
            .filter(|i| i.x > div.x && i.owner_name != "nanobar")
            .collect();

        if !hidden.is_empty() {
            println!(
                "\nwill hide ({}):",
                hidden.len()
            );
            for item in &hidden {
                println!("  - {}", item.owner_name);
            }
        }
        println!("\nvisible ({}):", visible.len());
        for item in &visible {
            println!("  - {}", item.owner_name);
        }
    }

    Ok(())
}
