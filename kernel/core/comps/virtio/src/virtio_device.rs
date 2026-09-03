// SPDX-License-Identifier: MPL-2.0

//! VirtIO devices in the primary device topology.

use alloc::{boxed::Box, format, sync::Arc};
use core::{
    hint::spin_loop,
    sync::atomic::{AtomicU32, Ordering},
};

use aster_device::IsChild;
use aster_pci::common_device::PciCommonDevice;
use aster_systree::{
    BranchNodeFields, SysAttrSet, SysObj, SysPerms, SysStr, inherit_sys_branch_node,
};
use bitflags::bitflags;
use ostd::{error, sync::SpinLock};

use crate::{
    VirtioDeviceType,
    device::{
        block::device::BlockDevice, console::device::ConsoleDevice,
        filesystem::device::FileSystemDevice, input::device::InputDevice,
        network::device::NetworkDevice, socket::device::SocketDevice,
    },
    transport::{DeviceStatus, DeviceTransport, VirtioTransport},
};

static NEXT_VIRTIO_DEVICE_ID: AtomicU32 = AtomicU32::new(0);

/// A VirtIO topology device below a PCI or MMIO transport device.
#[derive(Debug)]
pub(crate) struct VirtioDevice {
    fields: BranchNodeFields<dyn SysObj, Self>,
    device_type: VirtioDeviceType,
    transport: SpinLock<Option<DeviceTransport>>,
}

impl IsChild<PciCommonDevice> for VirtioDevice {}

impl VirtioDevice {
    pub(crate) fn new(transport: Box<dyn VirtioTransport>) -> Arc<Self> {
        let number = NEXT_VIRTIO_DEVICE_ID.fetch_add(1, Ordering::Relaxed);
        let name = format!("virtio{number}");
        let device_type = transport.device_type();

        Arc::new_cyclic(|weak_self| Self {
            fields: BranchNodeFields::new(
                SysStr::from(name),
                SysAttrSet::new_empty(),
                weak_self.clone(),
            ),
            device_type,
            transport: SpinLock::new(Some(DeviceTransport::new(transport))),
        })
    }

    pub(crate) fn device_type(&self) -> VirtioDeviceType {
        self.device_type
    }

    pub(crate) fn take_transport(&self) -> DeviceTransport {
        self.transport.lock().take().unwrap()
    }

    /// Performs the common VirtIO device initialization sequence.
    ///
    /// Follow VirtIO 1.3, 3.1.1 "Driver Requirements: Device Initialization".
    /// Reference: <https://docs.oasis-open.org/virtio/virtio/v1.3/virtio-v1.3.html#x1-1230001>
    pub(crate) fn initialize(&self) -> bool {
        let mut transport = self.transport.lock();
        let transport = transport.as_mut().unwrap();

        // Reset the device.
        transport
            .write_device_status(DeviceStatus::empty())
            .unwrap();
        while transport.read_device_status() != DeviceStatus::empty() {
            spin_loop();
        }

        // Set `ACKNOWLEDGE` to report that the guest OS has noticed the device.
        transport
            .write_device_status(DeviceStatus::ACKNOWLEDGE)
            .unwrap();

        // Set `DRIVER` to report that the guest OS knows how to drive the device.
        transport
            .write_device_status(DeviceStatus::ACKNOWLEDGE | DeviceStatus::DRIVER)
            .unwrap();

        self.negotiate_features(transport);

        if !transport.is_legacy_version() {
            // Set `FEATURES_OK` to report that feature negotiation is complete.
            let status =
                DeviceStatus::ACKNOWLEDGE | DeviceStatus::DRIVER | DeviceStatus::FEATURES_OK;
            transport.write_device_status(status).unwrap();
            let status = transport.read_device_status();
            if !status.contains(DeviceStatus::FEATURES_OK) {
                error!(
                    "Device rejected negotiated features, device type: {:?}",
                    self.device_type
                );
                transport
                    .write_device_status(status | DeviceStatus::FAILED)
                    .unwrap();
                return false;
            }
        }

        true
    }

    fn negotiate_features(&self, transport: &mut DeviceTransport) {
        let features = transport.read_device_features();
        let mask = ((1u64 << 24) - 1) | (((1u64 << 24) - 1) << 50);
        let device_specified_features = features & mask;
        let device_support_features = match self.device_type {
            VirtioDeviceType::Network => {
                NetworkDevice::negotiate_features(device_specified_features)
            }
            VirtioDeviceType::Block => BlockDevice::negotiate_features(device_specified_features),
            VirtioDeviceType::Input => InputDevice::negotiate_features(device_specified_features),
            VirtioDeviceType::Console => {
                ConsoleDevice::negotiate_features(device_specified_features)
            }
            VirtioDeviceType::Socket => SocketDevice::negotiate_features(device_specified_features),
            VirtioDeviceType::FileSystem => {
                FileSystemDevice::negotiate_features(device_specified_features)
            }
            _ => device_specified_features,
        };
        let mut support_feature = Feature::from_bits_truncate(features);
        support_feature.remove(Feature::RING_EVENT_IDX);
        transport
            .write_driver_features(features & (support_feature.bits | device_support_features))
            .unwrap();
    }
}

aster_device::impl_class_device_parent!(VirtioDevice, fields, pub(crate));

bitflags! {
    /// Device-independent VirtIO feature bits.
    struct Feature: u64 {
        const NOTIFY_ON_EMPTY   = 1 << 24;
        const ANY_LAYOUT        = 1 << 27;
        const RING_INDIRECT_DESC = 1 << 28;
        const RING_EVENT_IDX    = 1 << 29;
        const UNUSED            = 1 << 30;
        const VERSION_1         = 1 << 32;
        const ACCESS_PLATFORM   = 1 << 33;
        const RING_PACKED       = 1 << 34;
        const IN_ORDER          = 1 << 35;
        const ORDER_PLATFORM    = 1 << 36;
        const SR_IOV            = 1 << 37;
        const NOTIFICATION_DATA = 1 << 38;
        const NOTIF_CONFIG_DATA = 1 << 39;
        const RING_RESET        = 1 << 40;
    }
}

inherit_sys_branch_node!(VirtioDevice, fields, {
    fn perms(&self) -> SysPerms {
        SysPerms::DEFAULT_RO_PERMS
    }
});
