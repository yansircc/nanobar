use core_foundation::base::TCFType;
use core_foundation::dictionary::CFDictionaryRef;
use core_foundation::number::CFNumber;
use core_foundation::string::{CFString, CFStringRef};
use core_graphics::display::{
    kCGNullWindowID, CGWindowListCopyWindowInfo,
};
use std::ffi::c_void;

pub struct MenuBarItem {
    pub window_id: u32,
    pub owner_name: String,
    pub owner_pid: i32,
    pub x: f64,
    pub width: f64,
}

extern "C" {
    fn CFDictionaryGetValue(dict: CFDictionaryRef, key: *const c_void) -> *const c_void;
}

pub unsafe fn dict_get_string(dict: CFDictionaryRef, key: &str) -> Option<String> {
    let cf_key = CFString::new(key);
    let value = unsafe { CFDictionaryGetValue(dict, cf_key.as_CFTypeRef() as *const c_void) };
    if value.is_null() {
        return None;
    }
    let cf_str: CFString = unsafe { TCFType::wrap_under_get_rule(value as CFStringRef) };
    Some(cf_str.to_string())
}

pub unsafe fn dict_get_i64(dict: CFDictionaryRef, key: &str) -> Option<i64> {
    let cf_key = CFString::new(key);
    let value = unsafe { CFDictionaryGetValue(dict, cf_key.as_CFTypeRef() as *const c_void) };
    if value.is_null() {
        return None;
    }
    let cf_num: CFNumber =
        unsafe { TCFType::wrap_under_get_rule(value as core_foundation::number::CFNumberRef) };
    cf_num.to_i64()
}

pub unsafe fn dict_get_f64_from_rect(dict: CFDictionaryRef, rect_key: &str, field: &str) -> Option<f64> {
    let cf_key = CFString::new(rect_key);
    let value = unsafe { CFDictionaryGetValue(dict, cf_key.as_CFTypeRef() as *const c_void) };
    if value.is_null() {
        return None;
    }
    let bounds_dict = value as CFDictionaryRef;
    let field_key = CFString::new(field);
    let field_value =
        unsafe { CFDictionaryGetValue(bounds_dict, field_key.as_CFTypeRef() as *const c_void) };
    if field_value.is_null() {
        return None;
    }
    let cf_num: CFNumber = unsafe {
        TCFType::wrap_under_get_rule(field_value as core_foundation::number::CFNumberRef)
    };
    cf_num.to_f64()
}

pub fn list_menubar_items() -> Vec<MenuBarItem> {
    let mut items = Vec::new();

    // Use option 0 (all windows) to include items pushed off-screen by the divider
    let window_list = unsafe {
        CGWindowListCopyWindowInfo(0, kCGNullWindowID)
    };

    if window_list.is_null() {
        return items;
    }

    let count = unsafe { core_foundation::array::CFArrayGetCount(window_list as _) };

    for i in 0..count {
        let dict = unsafe {
            core_foundation::array::CFArrayGetValueAtIndex(window_list as _, i)
                as CFDictionaryRef
        };
        if dict.is_null() {
            continue;
        }

        // Filter: kCGWindowLayer == 25
        let layer = unsafe { dict_get_i64(dict, "kCGWindowLayer") };
        if layer != Some(25) {
            continue;
        }

        let window_id = unsafe { dict_get_i64(dict, "kCGWindowNumber") }.unwrap_or(0) as u32;
        let owner_name =
            unsafe { dict_get_string(dict, "kCGWindowOwnerName") }.unwrap_or_default();
        let owner_pid = unsafe { dict_get_i64(dict, "kCGWindowOwnerPID") }.unwrap_or(0) as i32;
        let x = unsafe { dict_get_f64_from_rect(dict, "kCGWindowBounds", "X") }.unwrap_or(0.0);
        let width =
            unsafe { dict_get_f64_from_rect(dict, "kCGWindowBounds", "Width") }.unwrap_or(0.0);

        items.push(MenuBarItem {
            window_id,
            owner_name,
            owner_pid,
            x,
            width,
        });
    }

    unsafe {
        core_foundation::base::CFRelease(window_list as _);
    }

    // Filter out items at X=0 (detached/invisible status items from other apps)
    // but keep items with X<0 (pushed off-screen by divider)
    items.retain(|i| i.x != 0.0);

    // Sort by x position (left to right)
    items.sort_by(|a, b| a.x.partial_cmp(&b.x).unwrap_or(std::cmp::Ordering::Equal));

    items
}

/// Get bundle identifier from a process PID using lsappinfo
pub fn get_bundle_id(pid: i32) -> Option<String> {
    let output = std::process::Command::new("lsappinfo")
        .args(["info", "-only", "bundleid", &format!("{}", pid)])
        .output()
        .ok()?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    // Format: "bundleid" = "com.example.App"
    stdout
        .split('"')
        .nth(3)
        .map(String::from)
}

/// Get the saved preferred position for a status item from app defaults
pub fn get_preferred_position(bundle_id: &str) -> Option<f64> {
    let output = std::process::Command::new("defaults")
        .args([
            "read",
            bundle_id,
            "NSStatusItem Preferred Position Item-0",
        ])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    stdout.trim().parse().ok()
}
