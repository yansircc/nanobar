use std::cell::{Cell, OnceCell};
use objc2::{define_class, msg_send, sel, rc::Retained, runtime::{AnyObject, ProtocolObject},
    DefinedClass, MainThreadOnly};
use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy, NSApplicationDelegate,
    NSMenu, NSMenuDelegate, NSMenuItem, NSStatusBar, NSStatusItem, NSVariableStatusItemLength};
use objc2_foundation::{ns_string, MainThreadMarker, NSNotification, NSObject, NSObjectProtocol};
extern "C" { fn kill(pid: i32, sig: i32) -> i32; fn fork() -> i32; fn setsid() -> i32; }
#[derive(Debug)] struct DaemonIvars {
    status_item: OnceCell<Retained<NSStatusItem>>, pusher_item: OnceCell<Retained<NSStatusItem>>,
    hidden: Cell<bool>,
}

define_class!(
    #[unsafe(super = NSObject)]
    #[thread_kind = MainThreadOnly]
    #[ivars = DaemonIvars]
    #[derive(Debug)]
    struct Delegate;
    unsafe impl NSObjectProtocol for Delegate {}
    unsafe impl NSApplicationDelegate for Delegate {
        #[unsafe(method(applicationDidFinishLaunching:))]
        fn did_finish_launching(&self, _: &NSNotification) {
            let mtm = self.mtm();
            let bar = NSStatusBar::systemStatusBar();
            let item = bar.statusItemWithLength(NSVariableStatusItemLength);
            item.setAutosaveName(Some(ns_string!("Item-0")));
            if let Some(b) = item.button(mtm) { b.setTitle(ns_string!("\u{203a}")); }
            let pusher = bar.statusItemWithLength(NSVariableStatusItemLength);
            pusher.setAutosaveName(Some(ns_string!("Pusher-0")));
            if let Some(b) = pusher.button(mtm) { b.setTitle(ns_string!("\u{200B}")); }
            let menu = NSMenu::new(mtm);
            let quit = unsafe { NSMenuItem::initWithTitle_action_keyEquivalent(
                NSMenuItem::alloc(mtm), ns_string!("Quit"), Some(sel!(terminate:)), ns_string!("")) };
            menu.addItem(&quit);
            menu.setDelegate(Some(ProtocolObject::from_ref(self as &Delegate)));
            item.setMenu(Some(&menu));
            self.ivars().status_item.set(item).unwrap();
            self.ivars().pusher_item.set(pusher).unwrap();
            let _ = std::fs::write(std::env::temp_dir().join("nanobar.pid"),
                std::process::id().to_string());
        }
        #[unsafe(method(applicationWillTerminate:))]
        fn will_terminate(&self, _: &NSNotification) {
            let _ = std::fs::remove_file(std::env::temp_dir().join("nanobar.pid"));
        }
    }
    unsafe impl NSMenuDelegate for Delegate {
        #[unsafe(method(menuWillOpen:))]
        fn menu_will_open(&self, menu: &NSMenu) {
            let mtm = self.mtm();
            let is_left: bool = unsafe {
                let e: *const AnyObject =
                    msg_send![&*NSApplication::sharedApplication(mtm), currentEvent];
                e.is_null() || { let b: isize = msg_send![e, buttonNumber]; b == 0 }
            };
            if is_left {
                menu.cancelTrackingWithoutAnimation();
                let hidden = self.ivars().hidden.get();
                let pusher = self.ivars().pusher_item.get().unwrap();
                let button = self.ivars().status_item.get().unwrap().button(mtm).unwrap();
                pusher.setLength(if hidden { NSVariableStatusItemLength } else { 10000.0 });
                button.setTitle(if hidden { ns_string!("\u{203a}") } else { ns_string!("\u{2039}") });
                self.ivars().hidden.set(!hidden);
            }
        }
    }
);
impl Delegate {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(DaemonIvars {
            status_item: OnceCell::new(), pusher_item: OnceCell::new(), hidden: Cell::new(false),
        });
        unsafe { msg_send![super(this), init] }
    }
}

fn main() {
    if std::env::args().count() > 1 {
        println!("nanobar {} - minimal macOS menu bar manager\nUsage: nanobar",
            env!("CARGO_PKG_VERSION"));
        return;
    }
    if std::fs::read_to_string(std::env::temp_dir().join("nanobar.pid")).ok()
        .and_then(|s| s.trim().parse::<i32>().ok())
        .is_some_and(|pid| unsafe { kill(pid, 0) } == 0)
    { eprintln!("nanobar: already running"); std::process::exit(1); }
    let pid = unsafe { fork() };
    if pid != 0 { std::process::exit(if pid > 0 { 0 } else { 1 }); }
    unsafe { setsid(); }
    let mtm = MainThreadMarker::new().unwrap();
    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
    let delegate = Delegate::new(mtm);
    app.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
    app.run();
}
