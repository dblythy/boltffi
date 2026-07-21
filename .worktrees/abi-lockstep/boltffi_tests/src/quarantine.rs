#[cfg(boltffi_pending_constants)]
use crate::FixtureStatus;
#[cfg(boltffi_pending_constants)]
use boltffi::*;

#[cfg(boltffi_pending_constants)]
#[export]
pub const FIXTURE_LIMIT: u32 = 42;

#[cfg(boltffi_pending_constants)]
#[export]
pub const FIXTURE_LABEL: &str = "fixture";

#[cfg(boltffi_pending_constants)]
#[export]
pub const FIXTURE_DEFAULT_STATUS: FixtureStatus = FixtureStatus::Pending;

#[cfg(boltffi_pending_closure_return)]
use boltffi::*;

#[cfg(boltffi_pending_closure_return)]
#[export]
pub fn try_make_adder(fail: bool) -> Result<Box<dyn Fn(u32) -> u32>, String> {
    if fail {
        Err("adder unavailable".to_string())
    } else {
        Ok(Box::new(|value| value + 1))
    }
}
