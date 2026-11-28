//! Keeps the display awake while the picture is live (#215): `SetThreadExecutionState` with
//! `ES_DISPLAY_REQUIRED`, held for as long as the value lives. Only the display's idle timeout:
//! the PC still sleeps when asked. The state belongs to the thread that set it, so the value is
//! made and dropped on the UI thread (it isn't `Send`).

use std::marker::PhantomData;
use windows::Win32::System::Power::{ES_CONTINUOUS, ES_DISPLAY_REQUIRED, SetThreadExecutionState};

pub struct DisplayAwake(PhantomData<*const ()>);

impl DisplayAwake {
    /// Sets the state, or `None` if Windows refused it (the display then sleeps as usual).
    pub fn hold() -> Option<Self> {
        // SAFETY: plain flags, no pointers.
        let previous = unsafe { SetThreadExecutionState(ES_CONTINUOUS | ES_DISPLAY_REQUIRED) };
        (previous.0 != 0).then_some(Self(PhantomData))
    }
}

impl Drop for DisplayAwake {
    fn drop(&mut self) {
        // SAFETY: as above; `ES_CONTINUOUS` alone clears what `hold` set on this thread.
        unsafe { SetThreadExecutionState(ES_CONTINUOUS) };
    }
}
