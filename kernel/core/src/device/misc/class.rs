// SPDX-License-Identifier: MPL-2.0

//! The misc device class and its sysfs device nodes.

use alloc::sync::Arc;

use aster_device::{Class, register_class};
use aster_systree::{BranchNodeFields, SysAttrSet, SysPerms, SysStr, inherit_sys_branch_node};
use spin::Once;

use super::{AnyMiscDevice, MiscClass};
const MISC_CLASS_NAME: &str = "misc";

static MISC_CLASS: Once<Arc<MiscClass>> = Once::new();

impl MiscClass {
    fn new() -> Arc<Self> {
        Arc::new_cyclic(|weak_self| Self {
            fields: BranchNodeFields::new(
                SysStr::from(MISC_CLASS_NAME),
                SysAttrSet::new_empty(),
                weak_self.clone(),
            ),
        })
    }

    fn add_device_link(&self, name: SysStr, path: &str) -> aster_systree::Result<()> {
        self.fields
            .add_child(aster_device::ClassDeviceLink::new(name, path))
    }
}

impl Class for MiscClass {
    type Device = dyn AnyMiscDevice;

    fn name() -> &'static str {
        MISC_CLASS_NAME
    }

    fn register(device: Arc<Self::Device>) -> aster_systree::Result<()> {
        let name = device.name().clone();
        let path = device.path();
        MISC_CLASS
            .get()
            .unwrap()
            .add_device_link(name, path.as_ref())
    }
}

inherit_sys_branch_node!(MiscClass, fields, {
    fn perms(&self) -> SysPerms {
        SysPerms::DEFAULT_RO_PERMS
    }
});

pub(super) fn init_in_first_kthread() {
    MISC_CLASS.call_once(|| register_class(MiscClass::new()).unwrap());
}
