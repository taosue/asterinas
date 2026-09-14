// SPDX-License-Identifier: MPL-2.0

//! Misc devices.
//!
//! Character device with major number 10.

use aster_device::AnyDevice;
use aster_systree::{BranchNodeFields, SysObj};
use device_id::MajorId;
use spin::Once;

use super::registry::char::{MajorIdOwner, acquire_major};

mod class;
mod hwrng;
#[cfg(all(target_arch = "x86_64", feature = "cvm_guest"))]
pub(crate) mod tdxguest;

static MISC_MAJOR: Once<MajorIdOwner> = Once::new();

#[derive(Debug)]
pub(crate) struct MiscClass {
    fields: BranchNodeFields<dyn SysObj, Self>,
}

/// A device accepted by the misc class.
pub(crate) trait AnyMiscDevice: AnyDevice<Class = MiscClass> {}

pub(super) fn init_in_first_kthread() {
    MISC_MAJOR.call_once(|| acquire_major(MajorId::new(10)).unwrap());
    class::init_in_first_kthread();

    hwrng::init_in_first_kthread();

    #[cfg(target_arch = "x86_64")]
    ostd::if_tdx_enabled!({
        tdxguest::init().unwrap();
    });
}
