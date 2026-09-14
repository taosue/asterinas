// SPDX-License-Identifier: MPL-2.0

//! The `/sys/devices/virtual` topology parent.

use aster_device::{AnyDevice, ClassFor, IsChild};
use aster_systree::{
    BranchNodeFields, SysAttrSet, SysObj, SysPerms, SysStr, inherit_sys_branch_node,
};
use spin::Once;

use crate::prelude::*;

/// Registers a device under the virtual topology.
pub(crate) fn register_device<T: AnyDevice + IsChild<VirtualBusDevice>>(
    device: Arc<T>,
) -> Result<()>
where
    T::Class: ClassFor<T>,
{
    VIRTUAL_BUS_DEVICE.get().unwrap().add_device(device)?;
    Ok(())
}

pub(super) fn init() {
    VIRTUAL_BUS_DEVICE
        .call_once(|| aster_device::register_device(VirtualBusDevice::new()).unwrap());
}

static VIRTUAL_BUS_DEVICE: Once<Arc<VirtualBusDevice>> = Once::new();

#[derive(Debug)]
pub(crate) struct VirtualBusDevice {
    fields: BranchNodeFields<dyn SysObj, Self>,
}

impl AnyDevice for VirtualBusDevice {}

impl VirtualBusDevice {
    fn new() -> Arc<Self> {
        Arc::new_cyclic(|weak_self| Self {
            fields: BranchNodeFields::new(
                SysStr::from("virtual"),
                SysAttrSet::new_empty(),
                weak_self.clone(),
            ),
        })
    }
}

aster_device::impl_device_parent!(VirtualBusDevice, fields);

inherit_sys_branch_node!(VirtualBusDevice, fields, {
    fn perms(&self) -> SysPerms {
        SysPerms::DEFAULT_RO_PERMS
    }
});
