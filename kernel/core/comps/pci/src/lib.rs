// SPDX-License-Identifier: MPL-2.0

//! The PCI bus of Asterinas.
//!
//! Users can implement [`PciDriver`] to initialize devices on the PCI bus.
//!
//! Use case:
//!
//! ```rust no_run
//! #[derive(Debug)]
//! pub struct PciDriverA {
//!     devices: Mutex<Vec<Arc<PciCommonDevice>>>,
//! }
//!
//! impl PciDriver for PciDriverA {
//!     fn probe(
//!         &self,
//!         device: &Arc<PciCommonDevice>,
//!     ) -> Result<(), BusProbeError> {
//!         if device.device_id().vendor_id != 0x1234 {
//!             return Err(BusProbeError::DeviceNotMatch);
//!         }
//!         self.devices.lock().push(device.clone());
//!         Ok(())
//!     }
//! }
//!
//! pub fn driver_a_init() {
//!     let driver_a = Arc::new(PciDriverA {
//!         devices: Mutex::new(Vec::new()),
//!     });
//!     pci::register_driver(driver_a);
//! }
//! ```

#![no_std]
#![deny(unsafe_code)]

// Set this crate's log prefix for `ostd::log`.
macro_rules! __log_prefix {
    () => {
        "pci: "
    };
}

#[cfg_attr(target_arch = "x86_64", path = "arch/x86/mod.rs")]
#[cfg_attr(target_arch = "riscv64", path = "arch/riscv/mod.rs")]
#[cfg_attr(target_arch = "loongarch64", path = "arch/loongarch/mod.rs")]
mod arch;

pub mod bus;
pub mod capability;
pub mod cfg_space;
pub mod common_device;
mod device_info;
mod root;

extern crate alloc;

use alloc::sync::Arc;

use component::{ComponentInitError, init_component};
pub use device_info::{PciDeviceId, PciDeviceLocation};
use root::PciRootDevice;
use spin::Once;

use self::{bus::PciDriver, common_device::PciCommonDevice};

#[init_component]
fn pci_init() -> Result<(), ComponentInitError> {
    init();
    Ok(())
}

/// Registers a PCI driver with the PCI root device.
pub fn register_driver(driver: Arc<dyn PciDriver>) {
    if let Some(root) = PCI_ROOT_DEVICE.get() {
        root.register_driver(driver);
    }
}

fn init() {
    let Some(all_bus) = arch::init() else {
        ostd::info!("no PCI bus was found");
        return;
    };
    ostd::info!("initializing the PCI bus with bus numbers `{:?}`", all_bus);

    let root = PCI_ROOT_DEVICE.call_once(|| {
        let root = PciRootDevice::new(*all_bus.start());
        aster_device::register_device(root.clone()).unwrap();
        root
    });

    let all_dev = PciDeviceLocation::MIN_DEVICE..=PciDeviceLocation::MAX_DEVICE;
    let all_func = PciDeviceLocation::MIN_FUNCTION..=PciDeviceLocation::MAX_FUNCTION;

    for bus in all_bus {
        for device in all_dev.clone() {
            let mut device_location = PciDeviceLocation {
                bus,
                device,
                function: PciDeviceLocation::MIN_FUNCTION,
            };

            let Some(first_function_device) = PciCommonDevice::new(device_location) else {
                continue;
            };
            let has_multi_function = first_function_device.has_multi_funcs();
            // Register function 0 in advance
            root.register_common_device(first_function_device).unwrap();

            if has_multi_function {
                for function in all_func.clone().skip(1) {
                    device_location.function = function;
                    if let Some(common_device) = PciCommonDevice::new(device_location) {
                        root.register_common_device(common_device).unwrap();
                    }
                }
            }
        }
    }
}

static PCI_ROOT_DEVICE: Once<Arc<PciRootDevice>> = Once::new();
