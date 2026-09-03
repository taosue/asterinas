// SPDX-License-Identifier: MPL-2.0

//! PCI bus

use alloc::{collections::VecDeque, sync::Arc, vec::Vec};
use core::fmt::Debug;

use ostd::{bus::BusProbeError, debug, error};

use super::PciCommonDevice;

/// A trait that represents PCI device drivers.
///
/// The PCI bus will pass the device through the `probe` function when a new device is registered.
pub trait PciDriver: Sync + Send + Debug {
    /// Probes an unclaimed PCI device.
    ///
    /// If the driver matches and succeeds in initializing the unclaimed device,
    /// then it returns `Ok(())`, signaling that the PCI device is now ready to work.
    ///
    /// Once a device is matched and claimed by a driver,
    /// it won't be fed to another driver for probing.
    fn probe(&self, device: &Arc<PciCommonDevice>) -> Result<(), BusProbeError>;
}

/// The PCI bus used to register PCI drivers and devices.
///
/// If a component wishes to drive a PCI device, it needs to provide the following:
/// 1. A [`PciDriver`] instance.
/// 2. Driver-owned storage for any state needed after probing succeeds.
#[derive(Debug)]
pub struct PciBus {
    devices: VecDeque<Arc<PciCommonDevice>>,
    drivers: Vec<Arc<dyn PciDriver>>,
}

impl PciBus {
    /// Registers a PCI driver to the PCI bus.
    pub fn register_driver(&mut self, driver: Arc<dyn PciDriver>) {
        debug!("Register PCI driver: {:#x?}", driver);
        let length = self.devices.len();
        for _ in (0..length).rev() {
            let device = self.devices.pop_front().unwrap();
            match driver.probe(&device) {
                Ok(()) => {
                    continue;
                }
                Err(err) => {
                    if err != BusProbeError::DeviceNotMatch {
                        error!("device construction failed, reason: {:?}", err);
                    }
                }
            }
            self.devices.push_back(device);
        }
        self.drivers.push(driver);
    }

    pub(super) fn register_common_device(&mut self, device: Arc<PciCommonDevice>) {
        debug!("found device: {:#x?}", device);
        for driver in self.drivers.iter() {
            match driver.probe(&device) {
                Ok(()) => {
                    return;
                }
                Err(err) => {
                    if err != BusProbeError::DeviceNotMatch {
                        error!("device construction failed, reason: {:?}", err);
                    }
                }
            }
        }
        self.devices.push_back(device);
    }

    pub(super) const fn new() -> Self {
        Self {
            devices: VecDeque::new(),
            drivers: Vec::new(),
        }
    }
}
