//! Keeps the display awake while the picture is live (#215): an IOKit power assertion,
//! `PreventUserIdleDisplaySleep`, held for as long as the value lives. Only the display's idle sleep:
//! the Mac still sleeps on a lid close or when asked.

use objc2_core_foundation::CFString;

#[link(name = "IOKit", kind = "framework")]
unsafe extern "C" {
    fn IOPMAssertionCreateWithName(assertion_type: *const CFString, level: u32, name: *const CFString, id: *mut u32) -> i32;
    fn IOPMAssertionRelease(id: u32) -> i32;
}

/// `kIOPMAssertionLevelOn`.
const LEVEL_ON: u32 = 255;

pub struct DisplayAwake(u32);

impl DisplayAwake {
    /// Takes the assertion, or `None` if macOS refused it (the display then sleeps as usual).
    pub fn hold() -> Option<Self> {
        let kind = CFString::from_static_str("PreventUserIdleDisplaySleep");
        let name = CFString::from_static_str("xReveal is showing the picture from your capture card");
        let mut id = 0;
        // SAFETY: both strings are CFStringRefs alive for the call; `id` is the u32 out-parameter.
        let status = unsafe { IOPMAssertionCreateWithName(&*kind, LEVEL_ON, &*name, &mut id) };
        (status == 0).then_some(Self(id))
    }
}

impl Drop for DisplayAwake {
    fn drop(&mut self) {
        // SAFETY: the id came from IOPMAssertionCreateWithName and is released once.
        unsafe { IOPMAssertionRelease(self.0) };
    }
}
