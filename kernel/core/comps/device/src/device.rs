// SPDX-License-Identifier: MPL-2.0

//! The primary `/sys/devices` hierarchy.

use alloc::sync::Arc;

use aster_systree::{
    BranchNodeFields, SysAttrSet, SysBranchNode, SysObj, SysPerms, SysStr, inherit_sys_branch_node,
};
use spin::Once;

use crate::{Class, NoClass};

/// A device with a statically selected class or [`crate::NoClass`].
pub trait AnyDevice: SysBranchNode {
    type Class: Class = NoClass;
}

/// Marks `Child` as a child of `Parent` in the device topology.
pub trait IsChild<Parent>: SysBranchNode {}

static DEVICE_ROOT: Once<Arc<DevicesRoot>> = Once::new();

/// Registers `device` directly below `/sys/devices`.
pub fn register_device<T: SysBranchNode>(device: Arc<T>) -> aster_systree::Result<Arc<T>> {
    DEVICE_ROOT.get().unwrap().add_device(device.clone())?;
    Ok(device)
}

/// Initializes the `/sys/devices` root.
pub(super) fn init() {
    DEVICE_ROOT.call_once(|| {
        let root = DevicesRoot::new();
        aster_systree::primary_tree()
            .root()
            .add_child(root.clone())
            .unwrap();
        root
    });
}

#[derive(Debug)]
struct DevicesRoot {
    fields: BranchNodeFields<dyn SysObj, Self>,
}

impl DevicesRoot {
    fn new() -> Arc<Self> {
        Arc::new_cyclic(|weak_self| Self {
            fields: BranchNodeFields::new(
                SysStr::from("devices"),
                SysAttrSet::new_empty(),
                weak_self.clone(),
            ),
        })
    }

    fn add_device(&self, device: Arc<dyn SysBranchNode>) -> aster_systree::Result<()> {
        self.fields.add_child(device)
    }
}

inherit_sys_branch_node!(DevicesRoot, fields, {
    fn perms(&self) -> SysPerms {
        SysPerms::DEFAULT_RO_PERMS
    }
});
