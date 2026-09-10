// SPDX-License-Identifier: MPL-2.0

#![no_std]
#![deny(unsafe_code)]
#![feature(trait_alias)]
mod buffer;
pub mod dma_pool;
mod driver;

extern crate alloc;
#[macro_use]
extern crate ostd_pod;

use alloc::{collections::BTreeMap, format, string::String, sync::Arc, vec::Vec};
use core::{
    any::Any,
    fmt::Debug,
    sync::atomic::{AtomicU32, Ordering},
};

use aster_bigtcp::device::DeviceCapabilities;
use aster_device::{AnyDevice, Class, ClassFor};
use aster_softirq::{
    BottomHalfDisabled, SoftIrqLine,
    softirq_id::{NETWORK_RX_SOFTIRQ_ID, NETWORK_TX_SOFTIRQ_ID},
};
use aster_systree::{
    BranchNodeFields, Result as SysResult, SysAttrSet, SysObj, SysPerms, SysStr,
    inherit_sys_branch_node,
};
pub use buffer::{RxBuffer, TxBuffer, TxBufferBuilder};
use component::{ComponentInitError, init_component};
pub use driver::NetworkDeviceAdapter;
use ostd::sync::SpinLock;
use spin::Once;

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub struct EthernetAddr(pub [u8; 6]);

#[derive(Clone, Copy, Debug)]
pub enum NetError {
    NotReady,
    Busy,
    NoMemory,
}

static NETWORK_CLASS: Once<Arc<NetworkClass>> = Once::new();

const NETWORK_CLASS_NAME: &str = "net";

/// The network device class under `/sys/class/net`.
pub struct NetworkClass {
    fields: BranchNodeFields<dyn SysObj, Self>,
    devices: SpinLock<BTreeMap<String, NetworkDeviceIrqCallbackSet>, BottomHalfDisabled>,
}

impl NetworkClass {
    fn new() -> Arc<Self> {
        Arc::new_cyclic(|weak_self| Self {
            fields: BranchNodeFields::new(
                SysStr::from(NETWORK_CLASS_NAME),
                SysAttrSet::new_empty(),
                weak_self.clone(),
            ),
            devices: SpinLock::new(BTreeMap::new()),
        })
    }

    /// Returns the registered network class.
    fn class() -> &'static Self {
        NETWORK_CLASS.get().unwrap().as_ref()
    }

    fn add_device_link(&self, name: SysStr, path: &str) -> SysResult<()> {
        self.fields
            .add_child(aster_device::ClassDeviceLink::new(name, path))
    }
}

impl Class for NetworkClass {
    type Device = dyn AnyNetworkDevice;

    fn name() -> &'static str {
        NETWORK_CLASS_NAME
    }

    fn register(device: Arc<Self::Device>) -> SysResult<()> {
        let class = NETWORK_CLASS.get().unwrap();
        let name = device.name().clone();
        let path = device.path();
        class.add_device_link(name, path.as_ref())?;
        class.devices.lock().insert(
            device.name().clone().into_owned(),
            NetworkDeviceIrqCallbackSet::new(device),
        );
        Ok(())
    }
}

impl<D: AnyNetworkDevice> ClassFor<D> for NetworkClass {
    fn into_class_device(device: Arc<D>) -> Arc<Self::Device> {
        device
    }
}

inherit_sys_branch_node!(NetworkClass, fields, {
    fn perms(&self) -> SysPerms {
        SysPerms::DEFAULT_RO_PERMS
    }
});

pub trait AnyNetworkDevice: AnyDevice + Send + Sync + Any + Debug {
    // ================Device Information=================

    fn mac_addr(&self) -> EthernetAddr;
    fn capabilities(&self) -> DeviceCapabilities;

    // ================Device Operation===================

    fn can_receive(&self) -> bool;
    fn can_send(&self) -> bool;

    /// Receives a packet from network. If packet is ready, returns a `RxBuffer` containing the packet.
    /// Otherwise, return [`NetError::NotReady`].
    fn receive(&self) -> Result<RxBuffer, NetError>;

    /// Sends a packet to network.
    fn send(&self, packet: &[u8]) -> Result<(), NetError>;

    /// Frees processes tx buffers.
    fn free_processed_tx_buffers(&self);

    /// Notifies the device driver that a polling operation has ended.
    ///
    /// The driver can assume that the device remains protected by acquiring a poll lock
    /// for the entire duration of the polling process.
    /// Thus two polling process cannot happen simultaneously.
    fn notify_poll_end(&self);
}

pub trait NetDeviceCallback = Fn() + Send + Sync + 'static;

pub fn get_device(name: &str) -> Option<Arc<dyn AnyNetworkDevice>> {
    NetworkClass::class()
        .devices
        .lock()
        .get(name)
        .map(|set| set.device.clone())
}

/// Registers callback which will be called when receiving message.
///
/// Since the callback will be called in softirq context,
/// the callback function should _not_ sleep.
pub fn register_recv_callback(name: &str, callback: impl NetDeviceCallback) {
    let devices = NetworkClass::class().devices.lock();
    let Some(device) = devices.get(name) else {
        return;
    };
    device.recv_callbacks.lock().push(Arc::new(callback));
}

/// Registers a callback that will be invoked
/// when the device has completed sending a packet.
///
/// Since this callback is executed in a softirq context,
/// the callback function should _not_ block or sleep.
///
/// Please note that the callback may not be called every time a packet is sent.
/// The driver may skip certain callbacks for performance optimization.
pub fn register_send_callback(name: &str, callback: impl NetDeviceCallback) {
    let devices = NetworkClass::class().devices.lock();
    let Some(device) = devices.get(name) else {
        return;
    };
    device.send_callbacks.lock().push(Arc::new(callback));
}

fn handle_rx_softirq() {
    let devices = NetworkClass::class().devices.lock();
    for callback_set in devices.values() {
        let recv_callbacks = callback_set.recv_callbacks.lock();
        for callback in recv_callbacks.iter() {
            callback();
        }
    }
}

fn handle_tx_softirq() {
    let devices = NetworkClass::class().devices.lock();
    for callback_set in devices.values() {
        callback_set.device.free_processed_tx_buffers();
        if !callback_set.device.can_send() {
            continue;
        }

        let send_callbacks = callback_set.send_callbacks.lock();
        for callback in send_callbacks.iter() {
            callback();
        }
    }
}

/// Raises softirq for handling transmission events
pub fn raise_send_softirq() {
    SoftIrqLine::get(NETWORK_TX_SOFTIRQ_ID).raise();
}

/// Raises softirq for handling reception events
pub fn raise_receive_softirq() {
    SoftIrqLine::get(NETWORK_RX_SOFTIRQ_ID).raise();
}

pub fn all_devices() -> Vec<(String, Arc<dyn AnyNetworkDevice>)> {
    NetworkClass::class()
        .devices
        .lock()
        .iter()
        .map(|(name, callbacks)| (name.clone(), callbacks.device.clone()))
        .collect()
}

#[init_component]
fn init() -> Result<(), ComponentInitError> {
    NETWORK_CLASS.call_once(|| aster_device::register_class(NetworkClass::new()).unwrap());

    SoftIrqLine::get(NETWORK_TX_SOFTIRQ_ID).enable(handle_tx_softirq);
    SoftIrqLine::get(NETWORK_RX_SOFTIRQ_ID).enable(handle_rx_softirq);

    Ok(())
}

type NetDeviceCallbackListRef = Arc<SpinLock<Vec<Arc<dyn NetDeviceCallback>>, BottomHalfDisabled>>;

struct NetworkDeviceIrqCallbackSet {
    device: Arc<dyn AnyNetworkDevice>,
    recv_callbacks: NetDeviceCallbackListRef,
    send_callbacks: NetDeviceCallbackListRef,
}

impl NetworkDeviceIrqCallbackSet {
    fn new(device: Arc<dyn AnyNetworkDevice>) -> Self {
        Self {
            device,
            recv_callbacks: Arc::new(SpinLock::new(Vec::new())),
            send_callbacks: Arc::new(SpinLock::new(Vec::new())),
        }
    }
}

/// Allocates the next network device name.
pub fn allocate_name() -> String {
    static NEXT_INDEX: AtomicU32 = AtomicU32::new(0);

    format!("eth{}", NEXT_INDEX.fetch_add(1, Ordering::Relaxed))
}

impl Debug for NetworkClass {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        f.debug_struct("NetworkClass")
            .field("fields", &self.fields)
            .field("device_count", &self.devices.lock().len())
            .finish()
    }
}
