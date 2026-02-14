use std::cell::OnceCell;
use std::ffi::c_void;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;
use std::path::PathBuf;
use std::sync::atomic::{AtomicPtr, AtomicU8, Ordering};

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2::{define_class, msg_send, sel, DefinedClass, MainThreadOnly};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSApplicationDelegate, NSMenu, NSMenuItem,
    NSStatusBar, NSStatusItem, NSVariableStatusItemLength,
};
use objc2_foundation::{ns_string, MainThreadMarker, NSNotification, NSObject, NSObjectProtocol, NSString};

// -- Global state for cross-thread communication --

/// Pending command: 0=none, 1=hide, 2=show, 3=stop
static PENDING_CMD: AtomicU8 = AtomicU8::new(0);
/// Current visibility: 0=shown, 1=hidden
static CURRENT_STATE: AtomicU8 = AtomicU8::new(0);
/// Raw pointer to the divider NSStatusItem (visible indicator, variable length)
static ITEM_PTR: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());
/// Raw pointer to the pusher NSStatusItem (invisible, expands to 10000pt to push items)
static PUSHER_PTR: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());
/// Raw pointer to the login NSMenuItem (Start at Login)
static LOGIN_ITEM_PTR: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());

// -- GCD FFI for dispatching to main thread --

extern "C" {
    static _dispatch_main_q: u8;
    fn dispatch_async_f(
        queue: *const u8,
        context: *mut c_void,
        work: unsafe extern "C" fn(*mut c_void),
    );
}

/// Callback executed on the main thread via dispatch_async_f.
/// Reads the pending command and adjusts the status items accordingly.
unsafe extern "C" fn process_on_main(_ctx: *mut c_void) {
    let cmd = PENDING_CMD.swap(0, Ordering::SeqCst);
    let item_ptr = ITEM_PTR.load(Ordering::SeqCst);
    let pusher_ptr = PUSHER_PTR.load(Ordering::SeqCst);
    if item_ptr.is_null() || pusher_ptr.is_null() {
        return;
    }

    // SAFETY: ptrs point to valid NSStatusItems created on the main thread.
    // This callback runs on the main thread (dispatched to main queue).
    // The items are kept alive by DaemonDelegate ivars.
    unsafe {
        let item = &*(item_ptr as *const NSStatusItem);
        let pusher = &*(pusher_ptr as *const NSStatusItem);
        let mtm = MainThreadMarker::new().unwrap();
        match cmd {
            1 => {
                // Hide: expand pusher to push items off screen, show indicator
                pusher.setLength(10000.0);
                if let Some(button) = item.button(mtm) {
                    button.setTitle(ns_string!("\u{2039}"));
                }
                CURRENT_STATE.store(1, Ordering::SeqCst);
            }
            2 => {
                // Show: collapse pusher, restore divider
                pusher.setLength(0.0);
                if let Some(button) = item.button(mtm) {
                    button.setTitle(ns_string!("\u{203a}"));
                }
                CURRENT_STATE.store(0, Ordering::SeqCst);
            }
            3 => {
                // Stop: clean up and exit
                pusher.setLength(0.0);
                CURRENT_STATE.store(0, Ordering::SeqCst);
                let _ = std::fs::remove_file(socket_path());
                let _ = std::fs::remove_file(pid_path());
                std::process::exit(0);
            }
            _ => {}
        }
    }
}

/// Update the login menu item title based on whether plist exists
unsafe fn update_login_item_title() {
    let ptr = LOGIN_ITEM_PTR.load(Ordering::SeqCst);
    if ptr.is_null() {
        return;
    }
    let login_item = unsafe { &*(ptr as *const NSMenuItem) };
    let title = if crate::is_installed() {
        NSString::from_str("Start at Login  \u{2713}")
    } else {
        NSString::from_str("Start at Login")
    };
    login_item.setTitle(&title);
}

/// Read the divider's saved preferred position from defaults
fn read_divider_position() -> Option<f64> {
    let output = std::process::Command::new("defaults")
        .args(["read", "nanobar", "NSStatusItem Preferred Position Item-0"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8_lossy(&output.stdout).trim().parse().ok()
}

/// Write the pusher's preferred position to defaults
fn write_pusher_position(pos: f64) {
    let _ = std::process::Command::new("defaults")
        .args([
            "write",
            "nanobar",
            "NSStatusItem Preferred Position Pusher-0",
            "-float",
            &format!("{:.1}", pos),
        ])
        .status();
}

// -- Paths --

pub fn socket_path() -> PathBuf {
    std::env::temp_dir().join("nanobar.sock")
}

pub fn pid_path() -> PathBuf {
    std::env::temp_dir().join("nanobar.pid")
}

// -- Socket listener (runs in background thread) --

fn socket_listener(path: PathBuf) {
    // Clean up stale socket
    let _ = std::fs::remove_file(&path);

    let listener = match UnixListener::bind(&path) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("nanobar: failed to bind socket: {}", e);
            std::process::exit(1);
        }
    };

    for stream in listener.incoming() {
        let stream = match stream {
            Ok(s) => s,
            Err(_) => continue,
        };

        let mut line = String::new();
        {
            let mut reader = BufReader::new(&stream);
            if reader.read_line(&mut line).is_err() {
                continue;
            }
        }

        let response = match line.trim() {
            "hide" => {
                PENDING_CMD.store(1, Ordering::SeqCst);
                unsafe {
                    dispatch_async_f(&_dispatch_main_q, std::ptr::null_mut(), process_on_main);
                }
                "ok\n"
            }
            "show" => {
                PENDING_CMD.store(2, Ordering::SeqCst);
                unsafe {
                    dispatch_async_f(&_dispatch_main_q, std::ptr::null_mut(), process_on_main);
                }
                "ok\n"
            }
            "stop" => {
                PENDING_CMD.store(3, Ordering::SeqCst);
                unsafe {
                    dispatch_async_f(&_dispatch_main_q, std::ptr::null_mut(), process_on_main);
                }
                "ok\n"
            }
            "ping" => "pong\n",
            "state" => {
                if CURRENT_STATE.load(Ordering::SeqCst) == 1 {
                    "hidden\n"
                } else {
                    "visible\n"
                }
            }
            _ => "unknown\n",
        };

        let _ = (&stream).write_all(response.as_bytes());
    }
}

// -- AppDelegate --

#[derive(Debug)]
struct DaemonIvars {
    status_item: OnceCell<Retained<NSStatusItem>>,
    pusher_item: OnceCell<Retained<NSStatusItem>>,
    menu: OnceCell<Retained<NSMenu>>,
    login_item: OnceCell<Retained<NSMenuItem>>,
}

define_class!(
    #[unsafe(super = NSObject)]
    #[thread_kind = MainThreadOnly]
    #[ivars = DaemonIvars]
    #[derive(Debug)]
    struct DaemonDelegate;

    unsafe impl NSObjectProtocol for DaemonDelegate {}

    unsafe impl NSApplicationDelegate for DaemonDelegate {
        #[unsafe(method(applicationDidFinishLaunching:))]
        fn did_finish_launching(&self, _notification: &NSNotification) {
            let mtm = self.mtm();
            let status_bar = NSStatusBar::systemStatusBar();

            // Create the divider status item (always visible, variable length)
            let status_item = status_bar.statusItemWithLength(NSVariableStatusItemLength);
            status_item.setAutosaveName(Some(ns_string!("Item-0")));

            if let Some(button) = status_item.button(mtm) {
                button.setTitle(ns_string!("\u{203a}"));
                // Left-click: toggle visibility via button action
                let _: () = unsafe { msg_send![&*button, setAction: sel!(toggleVisibility:)] };
                let _: () = unsafe { msg_send![&*button, setTarget: &*(self as *const DaemonDelegate as *const AnyObject)] };
            }

            // Create the pusher status item (invisible by default, expands to hide items)
            // Position it just to the LEFT of the divider so it pushes the correct items
            if let Some(divider_pos) = read_divider_position() {
                write_pusher_position(divider_pos + 2.0);
            }
            let pusher_item = status_bar.statusItemWithLength(0.0);
            pusher_item.setAutosaveName(Some(ns_string!("Pusher-0")));

            // Create the right-click menu
            let menu = NSMenu::new(mtm);

            // Start at Login item
            let login_title = if crate::is_installed() {
                NSString::from_str("Start at Login  \u{2713}")
            } else {
                NSString::from_str("Start at Login")
            };
            let login_item = unsafe {
                NSMenuItem::initWithTitle_action_keyEquivalent(
                    NSMenuItem::alloc(mtm),
                    &login_title,
                    Some(sel!(toggleStartAtLogin:)),
                    ns_string!(""),
                )
            };
            unsafe {
                login_item.setTarget(Some(&*(self as *const DaemonDelegate as *const AnyObject)));
            }
            menu.addItem(&login_item);

            // Separator
            menu.addItem(&NSMenuItem::separatorItem(mtm));

            // Quit item
            let quit_item = unsafe {
                NSMenuItem::initWithTitle_action_keyEquivalent(
                    NSMenuItem::alloc(mtm),
                    ns_string!("Quit"),
                    Some(sel!(quitApp:)),
                    ns_string!(""),
                )
            };
            unsafe {
                quit_item.setTarget(Some(&*(self as *const DaemonDelegate as *const AnyObject)));
            }
            menu.addItem(&quit_item);

            // Right-click context menu on the button (don't use status_item.setMenu
            // which would override the left-click action)
            if let Some(button) = status_item.button(mtm) {
                let _: () = unsafe { msg_send![&*button, setMenu: &*menu] };
            }

            // Store raw pointers for cross-thread access
            ITEM_PTR.store(
                Retained::as_ptr(&status_item) as *mut c_void,
                Ordering::SeqCst,
            );
            PUSHER_PTR.store(
                Retained::as_ptr(&pusher_item) as *mut c_void,
                Ordering::SeqCst,
            );
            LOGIN_ITEM_PTR.store(
                Retained::as_ptr(&login_item) as *mut c_void,
                Ordering::SeqCst,
            );

            // Keep alive in ivars
            self.ivars().status_item.set(status_item).unwrap();
            self.ivars().pusher_item.set(pusher_item).unwrap();
            self.ivars().menu.set(menu).unwrap();
            self.ivars().login_item.set(login_item).unwrap();

            // Write PID file
            let _ = std::fs::write(pid_path(), std::process::id().to_string());

            // Start socket listener
            let path = socket_path();
            std::thread::spawn(move || {
                socket_listener(path);
            });
        }
    }

    // -- Menu action methods --

    impl DaemonDelegate {
        #[unsafe(method(toggleVisibility:))]
        fn toggle_visibility(&self, _sender: *mut AnyObject) {
            let state = CURRENT_STATE.load(Ordering::SeqCst);
            if state == 1 {
                PENDING_CMD.store(2, Ordering::SeqCst);
            } else {
                PENDING_CMD.store(1, Ordering::SeqCst);
            }
            unsafe {
                process_on_main(std::ptr::null_mut());
            }
        }

        #[unsafe(method(toggleStartAtLogin:))]
        fn toggle_start_at_login(&self, _sender: *mut AnyObject) {
            if crate::is_installed() {
                let plist_path = crate::launchagent_path();
                let _ = std::process::Command::new("launchctl")
                    .args(["unload", &plist_path.to_string_lossy()])
                    .status();
                let _ = std::fs::remove_file(&plist_path);
            } else {
                if let Ok(exe) = std::env::current_exe() {
                    let plist_path = crate::launchagent_path();
                    if let Some(parent) = plist_path.parent() {
                        let _ = std::fs::create_dir_all(parent);
                    }
                    let plist = format!(
                        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>nanobar</string>
    <key>ProgramArguments</key>
    <array>
        <string>{}</string>
        <string>daemon</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
</dict>
</plist>
"#,
                        exe.to_string_lossy()
                    );
                    let _ = std::fs::write(&plist_path, plist);
                }
            }
            unsafe { update_login_item_title() };
        }

        #[unsafe(method(quitApp:))]
        fn quit_app(&self, _sender: *mut AnyObject) {
            let _ = std::fs::remove_file(socket_path());
            let _ = std::fs::remove_file(pid_path());
            std::process::exit(0);
        }
    }
);

impl DaemonDelegate {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(DaemonIvars {
            status_item: OnceCell::new(),
            pusher_item: OnceCell::new(),
            menu: OnceCell::new(),
            login_item: OnceCell::new(),
        });
        unsafe { msg_send![super(this), init] }
    }
}

// -- Entry point --

pub fn run_daemon() {
    let mtm = MainThreadMarker::new().unwrap();

    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);

    let delegate = DaemonDelegate::new(mtm);
    app.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));

    app.run();
}
