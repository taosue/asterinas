// SPDX-License-Identifier: MPL-2.0

use alloc::{boxed::Box, collections::vec_deque::VecDeque, sync::Arc};

use aster_pci::{bus::PciDriver, common_device::PciCommonDevice};
use ostd::{bus::BusProbeError, sync::SpinLock};

use super::device::VirtioPciModernTransport;
use crate::{
    transport::{VirtioTransport, pci::legacy::VirtioPciLegacyTransport},
    virtio_device::VirtioDevice,
};

#[derive(Debug)]
pub struct VirtioPciDriver {
    devices: SpinLock<VecDeque<Arc<VirtioDevice>>>,
}

impl VirtioPciDriver {
    pub fn pop_device(&self) -> Option<Arc<VirtioDevice>> {
        self.devices.lock().pop_front()
    }

    pub(super) fn new() -> Self {
        VirtioPciDriver {
            devices: SpinLock::new(VecDeque::new()),
        }
    }
}

impl PciDriver for VirtioPciDriver {
    fn probe(&self, device: &Arc<PciCommonDevice>) -> Result<(), BusProbeError> {
        const VIRTIO_DEVICE_VENDOR_ID: u16 = 0x1af4;
        if device.device_id().vendor_id != VIRTIO_DEVICE_VENDOR_ID {
            return Err(BusProbeError::DeviceNotMatch);
        }

        let has_vendor_cap = device.iter_vndr_capability().next().is_some();
        let device_id = *device.device_id();
        let transport: Box<dyn VirtioTransport> = match device_id.device_id {
            0x1000..0x1040 if (device.device_id().revision_id == 0) => {
                if has_vendor_cap {
                    let modern = VirtioPciModernTransport::new(device.clone())?;
                    Box::new(modern)
                } else {
                    let legacy = VirtioPciLegacyTransport::new(device.clone())?;
                    Box::new(legacy)
                }
            }
            0x1040..0x107f => {
                if !has_vendor_cap {
                    return Err(BusProbeError::DeviceNotMatch);
                }
                let modern = VirtioPciModernTransport::new(device.clone())?;
                Box::new(modern)
            }
            _ => return Err(BusProbeError::DeviceNotMatch),
        };
        let virtio_device = VirtioDevice::new(transport);
        device.add_device(virtio_device.clone()).unwrap();
        self.devices.lock().push_back(virtio_device);

        Ok(())
    }
}
