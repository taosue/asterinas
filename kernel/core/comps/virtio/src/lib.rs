// SPDX-License-Identifier: MPL-2.0

//! The virtio of Asterinas.
#![no_std]
#![deny(unsafe_code)]

extern crate alloc;
#[macro_use]
extern crate ostd_pod;

use alloc::sync::Arc;

use aster_block::MajorIdOwner;
use component::{ComponentInitError, init_component};
use device::{
    VirtioDeviceError, VirtioDeviceType, block::device::BlockDevice,
    console::device::ConsoleDevice, entropy::device::EntropyDevice,
    filesystem::device::FileSystemDevice, input::device::InputDevice,
    network::device::NetworkDevice, socket::device::SocketDevice,
};
use ostd::{error, warn};
use spin::Once;
use transport::{mmio::VIRTIO_MMIO_DRIVER, pci::VIRTIO_PCI_DRIVER};

// Set this crate's log prefix for `ostd::log`.
macro_rules! __log_prefix {
    () => {
        "virtio: "
    };
}

pub mod device;
mod dma_buf;
mod id_alloc;
mod queue;
mod transport;
mod virtio_device;
pub(crate) use virtio_device::VirtioDevice;

static VIRTIO_BLOCK_MAJOR_ID: Once<MajorIdOwner> = Once::new();

#[init_component]
fn virtio_component_init() -> Result<(), ComponentInitError> {
    VIRTIO_BLOCK_MAJOR_ID.call_once(|| aster_block::allocate_major().unwrap());

    // Find all devices and register them to the corresponding crate
    transport::init();

    device::entropy::init();
    device::network::init();
    device::socket::init();

    while let Some(device) = pop_device() {
        if !device.initialize() {
            continue;
        }

        let device_type = device.device_type();
        let res = match device_type {
            VirtioDeviceType::Block => {
                BlockDevice::new(device.take_transport()).and_then(|block_device| {
                    device
                        .add_class_device::<aster_block::BlockClass>(block_device)
                        .map_err(VirtioDeviceError::Sysfs)
                })
            }
            VirtioDeviceType::Console => ConsoleDevice::init(device.take_transport()),
            VirtioDeviceType::Entropy => EntropyDevice::init(device.take_transport()),
            VirtioDeviceType::Input => InputDevice::init(device.take_transport()),
            VirtioDeviceType::Network => {
                NetworkDevice::new(device.clone()).and_then(|network_device| {
                    device
                        .add_class_device::<aster_network::NetworkClass>(network_device)
                        .map_err(VirtioDeviceError::Sysfs)
                })
            }
            VirtioDeviceType::Socket => SocketDevice::init(device.take_transport()),
            VirtioDeviceType::FileSystem => FileSystemDevice::init(device.take_transport()),
            _ => {
                warn!("Found unimplemented device: {:?}", device_type);
                Ok(())
            }
        };
        if res.is_err() {
            error!(
                "Device initialization error: {:?}, device type: {:?}",
                res, device_type
            );
        }
    }
    Ok(())
}

fn pop_device() -> Option<Arc<VirtioDevice>> {
    if let Some(device) = VIRTIO_PCI_DRIVER.get().unwrap().pop_device() {
        return Some(device);
    }
    VIRTIO_MMIO_DRIVER.get().unwrap().pop_device_transport()
}
