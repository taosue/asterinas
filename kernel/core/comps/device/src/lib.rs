// SPDX-License-Identifier: MPL-2.0

//! The device model and its sysfs representation.
//!
//! [`ClassDevice`] combines a typed class payload with a shared [`DeviceBase`].
//! [`AnyDevice`] provides the erased view used by parent links and registration.
//! This component owns `/sys/devices` and `/sys/class`; sysfs displays their
//! nodes through `aster-systree`. Only [`add`] and [`remove`] edit device trees.

#![no_std]
#![deny(unsafe_code)]

extern crate alloc;

mod attr;
mod class;
mod device;
mod error;
mod node;
#[cfg(ktest)]
mod test;

use alloc::{sync::Arc, vec::Vec};

use aster_systree::{SysBranchNode, SysObj};
use component::{ComponentInitError, init_component};
use ostd::sync::Mutex;
use spin::Once;

pub use self::{
    attr::{Attr, ShowFn, StoreFn},
    class::{Class, ClassHandle, register_class},
    device::{
        AnyDevice, ClassDevice, ClassDeviceBuilder, DeviceBase, DeviceBuilder, Subsystem, add,
        remove,
    },
    error::{Error, Result},
    node::Container,
};
use self::{
    device::SubsystemOps,
    node::{Dir, GlueDirs, SysTreeEdit},
};

/// An owned string or a static string reference.
pub type SysStr = aster_systree::SysStr;

struct Registry {
    virtual_dir: Arc<Dir>,
    class: Arc<Dir>,
    virtual_glue_dirs: GlueDirs,
    subsystems: Mutex<Vec<Arc<dyn SubsystemOps>>>,
    lifecycle: Mutex<()>,
}

impl Registry {
    fn new() -> Result<Self> {
        let devices = Dir::new(SysStr::from("devices"));
        let virtual_dir = Dir::new(SysStr::from("virtual"));
        let class = Dir::new(SysStr::from("class"));
        devices.attach_child(virtual_dir.clone())?;
        let root = aster_systree::primary_tree().root();
        root.add_child(devices.clone())?;
        if let Err(error) = root.add_child(class.clone()) {
            let _ = root.remove_child("devices");
            return Err(error.into());
        }
        Ok(Self {
            virtual_dir,
            class,
            virtual_glue_dirs: GlueDirs::new(),
            subsystems: Mutex::new(Vec::new()),
            lifecycle: Mutex::new(()),
        })
    }
}

static REGISTRY: Once<Registry> = Once::new();

fn registry() -> &'static Registry {
    REGISTRY.get().expect("the device model is not initialized")
}

#[init_component]
fn init() -> core::result::Result<(), ComponentInitError> {
    REGISTRY.call_once(|| Registry::new().expect("cannot create device model roots"));
    Ok(())
}

/// Initializes the component for kernel tests.
#[cfg(ktest)]
pub fn init_for_ktest() {
    aster_systree::init_for_ktest();
    REGISTRY.call_once(|| Registry::new().expect("cannot create device model roots"));
}

/// Attaches a legacy sysfs node under `/sys/devices/virtual/<class>`.
///
/// This bridge preserves the TDX measurement directory until it can become a
/// class device with named binary attributes. New devices should use [`add`].
pub fn attach_to_virtual_glue_dir(class: &str, node: Arc<dyn SysObj>) -> Result<()> {
    let registry = registry();
    let _guard = registry.lifecycle.lock();
    registry
        .virtual_glue_dirs
        .attach_into(class, registry.virtual_dir.as_ref(), node)?;
    Ok(())
}
