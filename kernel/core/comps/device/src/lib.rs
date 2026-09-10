// SPDX-License-Identifier: MPL-2.0

//! Common device-model objects shared by low-level kernel components.

#![no_std]
#![deny(unsafe_code)]
#![feature(associated_type_defaults)]

extern crate alloc;

mod class;
mod dev;
mod device;

pub use class::{Class, ClassDeviceLink, ClassFor, ClassGlueDir, NoClass, register_class};
use component::{ComponentInitError, init_component};
pub use dev::{DeviceType, register_dev_node};
pub use device::{AnyDevice, IsChild, register_device};

#[init_component]
fn init() -> Result<(), ComponentInitError> {
    device::init();
    class::init();
    dev::init();
    Ok(())
}
