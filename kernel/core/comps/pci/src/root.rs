// SPDX-License-Identifier: MPL-2.0

//! PCI devices in the primary `/sys/devices` topology.

use alloc::{format, sync::Arc};

use aster_systree::{
    BranchNodeFields, SysAttrSet, SysObj, SysPerms, SysStr, inherit_sys_branch_node,
};
use ostd::sync::Mutex;

use super::{
    PciCommonDevice,
    bus::{PciBus, PciDriver},
};

/// The sysfs device representing a PCI root bus.
#[derive(Debug)]
pub(super) struct PciRootDevice {
    fields: BranchNodeFields<dyn SysObj, Self>,
    bus: Mutex<PciBus>,
}

impl PciRootDevice {
    pub(super) fn new(bus: u8) -> Arc<Self> {
        Arc::new_cyclic(|weak_self| Self {
            fields: BranchNodeFields::new(
                SysStr::from(format!("pci0000:{bus:02x}")),
                SysAttrSet::new_empty(),
                weak_self.clone(),
            ),
            bus: Mutex::new(PciBus::new()),
        })
    }

    pub(super) fn register_common_device(
        &self,
        device: Arc<PciCommonDevice>,
    ) -> aster_systree::Result<()> {
        let mut bus = self.bus.lock();
        bus.register_common_device(device.clone());
        self.fields.add_child(device)?;
        Ok(())
    }

    pub(super) fn register_driver(&self, driver: Arc<dyn PciDriver>) {
        self.bus.lock().register_driver(driver);
    }
}

inherit_sys_branch_node!(PciRootDevice, fields, {
    fn perms(&self) -> SysPerms {
        SysPerms::DEFAULT_RO_PERMS
    }
});
