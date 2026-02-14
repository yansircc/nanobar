use std::cell::OnceCell;
use std::ffi::c_void;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;
use std::path::PathBuf;
use std::sync::atomic::{AtomicPtr, AtomicU8, Ordering};

use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2::{define_class, msg_send, DefinedClass, MainThreadOnly};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSApplicationDelegate, NSStatusBar,
    NSStatusItem, NSVariableStatusItemLength,
};
use objc2_foundation::{ns_string, MainThreadMarker, NSNotification, NSObject, NSObjectProtocol};

// -- Global state for cross-thread communication --

/// Pending command: 0=none, 1=hide, 2=show, 3=stop
static PENDING_CMD: AtomicU8 = AtomicU8::new(0);
/// Current visibility: 0=shown, 1=hidden
static CURRENT_STATE: AtomicU8 = AtomicU8::new(0);
/// Raw pointer to NSStatusItem (set once from main thread, read from dispatch callback)
static ITEM_PTR: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());

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
/// Reads the pending command and adjusts the NSStatusItem accordingly.
unsafe extern "C" fn process_on_main(_ctx: *mut c_void) {
    let cmd = PENDING_CMD.swap(0, Ordering::SeqCst);
    let ptr = ITEM_PTR.load(Ordering::SeqCst);
    if ptr.is_null() {
        return;
    }

    // SAFETY: ptr points to a valid NSStatusItem created on the main thread.
    // This callback runs on the main thread (dispatched to main queue).
    // The item is kept alive by DaemonDelegate ivars.
    unsafe {
        let item = &*(ptr as *const NSStatusItem);
        let mtm = MainThreadMarker::new().unwrap();
        match cmd {
            1 => {
                // Hide: expand to push items off screen
                item.setLength(10000.0);
                if let Some(button) = item.button(mtm) {
                    button.setTitle(ns_string!(""));
                }
                CURRENT_STATE.store(1, Ordering::SeqCst);
            }
            2 => {
                // Show: contract back
                item.setLength(NSVariableStatusItemLength);
                if let Some(button) = item.button(mtm) {
                    button.setTitle(ns_string!("|"));
                }
                CURRENT_STATE.store(0, Ordering::SeqCst);
            }
            3 => {
                // Stop: clean up and exit
                item.setLength(NSVariableStatusItemLength);
                CURRENT_STATE.store(0, Ordering::SeqCst);
                let _ = std::fs::remove_file(socket_path());
                let _ = std::fs::remove_file(pid_path());
                std::process::exit(0);
            }
            _ => {}
        }
    }
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

            // Create status item (the divider)
            let status_bar = NSStatusBar::systemStatusBar();
            let status_item = status_bar.statusItemWithLength(NSVariableStatusItemLength);

            // Set autosaveName so macOS persists the position across restarts
            status_item.setAutosaveName(Some(ns_string!("Item-0")));

            if let Some(button) = status_item.button(mtm) {
                button.setTitle(ns_string!("|"));
            }

            // Store raw pointer for cross-thread dispatch
            ITEM_PTR.store(
                Retained::as_ptr(&status_item) as *mut c_void,
                Ordering::SeqCst,
            );

            // Keep alive in ivars
            self.ivars().status_item.set(status_item).unwrap();

            // Write PID file
            let _ = std::fs::write(pid_path(), std::process::id().to_string());

            // Start socket listener
            let path = socket_path();
            std::thread::spawn(move || {
                socket_listener(path);
            });
        }
    }
);

impl DaemonDelegate {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(DaemonIvars {
            status_item: OnceCell::new(),
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
