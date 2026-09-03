// SPDX-License-Identifier: MPL-2.0

use alloc::{sync::Arc, vec::Vec};

use aster_pci::{bus::PciDriver, common_device::PciCommonDevice};
use ostd::{bus::BusProbeError, sync::SpinLock};

use super::transport::NvmePciTransport;

#[derive(Debug)]
pub(crate) struct NvmePciDriver {
    devices: SpinLock<Vec<NvmePciTransport>>,
}

impl NvmePciDriver {
    pub(crate) fn pop_device_transport(&self) -> Option<NvmePciTransport> {
        self.devices.lock().pop()
    }

    pub(super) fn new() -> Self {
        NvmePciDriver {
            devices: SpinLock::new(Vec::new()),
        }
    }
}

impl PciDriver for NvmePciDriver {
    fn probe(&self, device: &Arc<PciCommonDevice>) -> Result<(), BusProbeError> {
        const NVME_DEVICE_CLASS: u8 = 0x01;
        const NVME_DEVICE_SUBCLASS: u8 = 0x08;
        const NVME_DEVICE_PROG_IF: u8 = 0x02;

        if device.device_id().class != NVME_DEVICE_CLASS
            || device.device_id().subclass != NVME_DEVICE_SUBCLASS
            || device.device_id().prog_if != NVME_DEVICE_PROG_IF
        {
            return Err(BusProbeError::DeviceNotMatch);
        }

        let transport = NvmePciTransport::new(device.clone())?;

        self.devices.lock().push(transport);

        Ok(())
    }
}
