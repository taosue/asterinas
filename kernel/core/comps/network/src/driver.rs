// SPDX-License-Identifier: MPL-2.0

use alloc::{sync::Arc, vec};

use aster_bigtcp::{
    device::{self, NotifyDevice, WithDevice},
    time::Instant,
};
use ostd::mm::VmWriter;

use crate::{AnyNetworkDevice, buffer::RxBuffer};

/// An adapter that gives the network stack mutable access to a shared network device.
#[derive(Clone, Debug)]
pub struct NetworkDeviceAdapter(pub Arc<dyn AnyNetworkDevice>);

impl WithDevice for NetworkDeviceAdapter {
    type Device = Self;

    fn with<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&mut Self::Device) -> R,
    {
        let mut device = self.clone();
        f(&mut device)
    }
}

impl device::Device for NetworkDeviceAdapter {
    type RxToken<'a> = RxToken;
    type TxToken<'a> = TxToken;

    fn receive(&mut self, _timestamp: Instant) -> Option<(Self::RxToken<'_>, Self::TxToken<'_>)> {
        if self.0.can_receive() && self.0.can_send() {
            let rx_buffer = self.0.receive().ok()?;
            Some((RxToken(rx_buffer), TxToken(self.0.clone())))
        } else {
            None
        }
    }

    fn transmit(&mut self, _timestamp: Instant) -> Option<Self::TxToken<'_>> {
        if self.0.can_send() {
            Some(TxToken(self.0.clone()))
        } else {
            None
        }
    }

    fn capabilities(&self) -> device::DeviceCapabilities {
        self.0.capabilities()
    }
}

impl NotifyDevice for NetworkDeviceAdapter {
    fn notify_poll_end(&mut self) {
        self.0.notify_poll_end();
    }
}

pub struct RxToken(RxBuffer);

impl device::RxToken for RxToken {
    fn consume<R, F>(self, f: F) -> R
    where
        F: FnOnce(&[u8]) -> R,
    {
        let mut payload = self.0.payload();
        let mut buffer = vec![0u8; payload.remain()];
        payload.read(&mut VmWriter::from(&mut buffer as &mut [u8]));
        f(&buffer)
    }
}

pub struct TxToken(Arc<dyn AnyNetworkDevice>);

impl device::TxToken for TxToken {
    fn consume<R, F>(self, len: usize, f: F) -> R
    where
        F: FnOnce(&mut [u8]) -> R,
    {
        let mut buffer = vec![0u8; len];
        let res = f(&mut buffer);
        self.0.send(&buffer).expect("Send packet failed");
        res
    }
}
