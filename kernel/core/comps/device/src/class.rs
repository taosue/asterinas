// SPDX-License-Identifier: MPL-2.0

//! Classes group devices by their user-visible function.
//!
//! [`Class`] supplies a typed payload and common attributes. A registered
//! [`ClassHandle`] owns the class directory and its registered members.

use alloc::{sync::Arc, vec::Vec};

use aster_systree::SysObj;
use ostd::sync::Mutex;

use crate::{
    AnyDevice, Attr, ClassDevice, DevNode, Result, SysStr,
    device::SubsystemOps,
    node::{Dir, SysTreeEdit},
};

/// A class of devices with a shared payload type and attribute declarations.
///
/// Callbacks run during registration and must not register classes or add or
/// remove devices themselves.
pub trait Class: Sized + Send + Sync + 'static {
    /// The directory name under `/sys/class`.
    const NAME: &'static str;
    /// The data carried by each device in this class.
    type Device: Send + Sync + 'static;

    /// Overrides the device-node path or initial permissions.
    fn devnode(&self, _dev: &ClassDevice<Self>) -> Option<DevNode> {
        None
    }

    /// Returns text attributes shared by all devices in this class.
    fn dev_attrs(&self) -> &'static [Attr<ClassDevice<Self>>] {
        &[]
    }
}

/// A registered class and its current devices.
pub struct ClassHandle<C: Class> {
    class: C,
    dir: Arc<Dir>,
    devices: Mutex<Vec<Arc<ClassDevice<C>>>>,
}

impl<C: Class> core::fmt::Debug for ClassHandle<C> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ClassHandle")
            .field("name", &C::NAME)
            .finish()
    }
}

/// Registers a class for the lifetime of the kernel.
pub fn register_class<C: Class>(class: C) -> Result<Arc<ClassHandle<C>>> {
    let registry = crate::registry();
    let _guard = registry.lifecycle.lock();
    let dir = Dir::new(SysStr::from(C::NAME));
    registry.class.attach_child(dir.clone())?;
    let handle = Arc::new(ClassHandle {
        class,
        dir,
        devices: Mutex::new(Vec::new()),
    });
    registry.subsystems.lock().push(handle.clone());
    Ok(handle)
}

impl<C: Class> ClassHandle<C> {
    /// Returns the class implementation.
    pub fn class(&self) -> &C {
        &self.class
    }

    /// Returns a snapshot of the registered members.
    pub fn devices(&self) -> Vec<Arc<ClassDevice<C>>> {
        self.devices.lock().clone()
    }
}

impl<C: Class> SubsystemOps for ClassHandle<C> {
    fn name(&self) -> &'static str {
        C::NAME
    }
    fn dir(&self) -> Arc<Dir> {
        self.dir.clone()
    }
    fn index_dir(&self) -> Arc<Dir> {
        self.dir.clone()
    }

    fn on_added(&self, dev: &Arc<dyn AnyDevice>) {
        let dev = dev
            .as_any()
            .downcast_ref::<ClassDevice<C>>()
            .expect("a device reports its own class as its subsystem");
        self.devices.lock().push(dev.this());
    }

    fn on_removed(&self, dev: &Arc<dyn AnyDevice>) {
        self.devices.lock().retain(|member| member.id() != dev.id());
    }
}
