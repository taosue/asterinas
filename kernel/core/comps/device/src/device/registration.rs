// SPDX-License-Identifier: MPL-2.0

//! Device construction and the shared registration lifecycle.

use alloc::{string::ToString, sync::Arc};

use aster_systree::SysObj;

use super::{AnyDevice, State, TreeParent};
use crate::{
    Attr, DevNodeRequest, DevNum, Error, Result, SysStr,
    attr::TyErasedAttr,
    devnum, hooks,
    node::{self, SysTreeEdit},
};

/// Builds a device with typed payload and attributes.
///
/// Construction does not publish the device. Call [`add`] to register it.
pub struct DeviceBuilder<H, P, D: ?Sized + 'static> {
    pub(super) handle: H,
    pub(super) payload: P,
    pub(super) name: SysStr,
    pub(super) parent: Option<Arc<dyn AnyDevice>>,
    pub(super) devnum: Option<DevNum>,
    pub(super) attrs: &'static [Attr<D>],
}

impl<H, P, D: ?Sized + 'static> DeviceBuilder<H, P, D> {
    pub(super) fn new(handle: H, name: SysStr, payload: P) -> Self {
        Self {
            handle,
            payload,
            name,
            parent: None,
            devnum: None,
            attrs: &[],
        }
    }

    /// Sets the device number used by `/sys/dev` and the `/dev` node.
    pub fn devnum(mut self, devnum: DevNum) -> Self {
        self.devnum = Some(devnum);
        self
    }

    /// Sets the parent, which must be registered before this device.
    pub fn parent(mut self, parent: Arc<dyn AnyDevice>) -> Self {
        self.parent = Some(parent);
        self
    }

    /// Sets the device's own attributes, in addition to its class attributes.
    pub fn attrs(mut self, attrs: &'static [Attr<D>]) -> Self {
        self.attrs = attrs;
        self
    }
}

/// Registers a device and publishes its sysfs directory and links.
///
/// A failed registration leaves no directory or links behind. A device can
/// be registered only once; construct a new device after failure or removal.
pub fn add<D: AnyDevice + ?Sized>(dev: &Arc<D>) -> Result<()> {
    let dev = dev.to_arc();
    let registry = crate::registry();
    // Lock order: lifecycle -> device state / directory children / class members.
    // Serializing the topology prevents parent removal from overtaking child add.
    let _guard = registry.lifecycle.lock();
    let base = dev.base();
    if *base.state.lock() != State::Initialized {
        return Err(Error::AlreadyAdded);
    }
    let mut registration = PendingRegistration {
        dev: &dev,
        is_committed: false,
    };
    let mut attrs = dev.attributes();
    if base.devnum().is_some() {
        attrs.push(TyErasedAttr::from_dyn(&DEV_ATTR));
    }
    base.attrs.add(attrs)?;

    if let Some(parent) = base.parent() {
        if !parent.base().is_added() {
            return Err(Error::ParentNotAdded);
        }
        parent.base().attach_child(dev.clone())?;
        base.tree_parent
            .call_once(|| TreeParent::Device(Arc::downgrade(parent)));
    } else {
        let dir = registry.virtual_glue_dirs.attach_into(
            dev.subsystem().name(),
            registry.virtual_dir.as_ref(),
            dev.clone(),
        )?;
        base.tree_parent
            .call_once(|| TreeParent::Dir(Arc::downgrade(&dir)));
    }

    let subsystem = dev.subsystem();
    node::add_link(base, "subsystem", &subsystem.ops.dir().path())?;
    base.links.lock().subsystem = true;
    if let Some(parent) = base.parent() {
        node::add_link(base, "device", &parent.path())?;
        base.links.lock().device = true;
    }
    node::add_link(subsystem.ops.index_dir().as_ref(), base.name(), &dev.path())?;
    base.links.lock().index = true;

    if let Some(devnum) = base.devnum() {
        node::add_link(
            registry.dev_index(devnum.kind()).as_ref(),
            &devnum.to_string(),
            &dev.path(),
        )?;
        base.links.lock().dev_index = true;
        let policy = dev.devnode_override().unwrap_or_default();
        let path = policy
            .path
            .unwrap_or_else(|| base.name().replace('!', "/").into());
        let request = DevNodeRequest::new(
            devnum,
            path,
            policy.mode.unwrap_or(devnum::DEFAULT_DEVNODE_MODE),
        )?;
        hooks::create_devnode(&request)?;
        // Cache the successful request: deletion must not call a possibly
        // stateful class policy a second time.
        base.devnode.call_once(|| request);
    }

    *base.state.lock() = State::Added;
    subsystem.ops.on_added(&dev);
    if let Some(parent) = base.parent() {
        parent
            .base()
            .child_devices
            .lock()
            .push(Arc::downgrade(&dev));
    }
    registration.is_committed = true;
    Ok(())
}

/// Removes a device after all its child devices have been removed.
///
/// Existing references keep the object alive, but attribute access fails.
pub fn remove<D: AnyDevice + ?Sized>(dev: &Arc<D>) -> Result<()> {
    let dev = dev.to_arc();
    let _guard = crate::registry().lifecycle.lock();
    let base = dev.base();
    if !base.is_added() {
        return Err(Error::NotAdded);
    }
    if !base.child_devices.lock().is_empty() {
        return Err(Error::HasChildren);
    }
    if let Some(request) = base.devnode.get() {
        hooks::delete_devnode(request)?;
    }
    *base.state.lock() = State::Removed;
    if let Some(parent) = base.parent() {
        parent.base().child_devices.lock().retain(|child| {
            child
                .upgrade()
                .is_some_and(|child| !Arc::ptr_eq(&child, &dev))
        });
    }
    dev.subsystem().ops.on_removed(&dev);
    detach(&dev);
    Ok(())
}

/// Rolls back every published resource unless registration succeeds.
struct PendingRegistration<'a> {
    dev: &'a Arc<dyn AnyDevice>,
    is_committed: bool,
}

impl Drop for PendingRegistration<'_> {
    fn drop(&mut self) {
        if !self.is_committed {
            *self.dev.base().state.lock() = State::Removed;
            detach(self.dev);
        }
    }
}

fn detach(dev: &Arc<dyn AnyDevice>) {
    let base = dev.base();
    let subsystem = dev.subsystem();
    let links = base.links.lock();
    // Remove only resources acquired by this attempt, preserving conflicting
    // entries owned by an existing device.
    if links.dev_index {
        let devnum = base
            .devnum()
            .expect("a device-number link requires a number");
        node::remove_link(
            crate::registry().dev_index(devnum.kind()).as_ref(),
            &devnum.to_string(),
        );
    }
    if links.index {
        node::remove_link(subsystem.ops.index_dir().as_ref(), base.name());
    }
    if links.device {
        node::remove_link(base, "device");
    }
    if links.subsystem {
        node::remove_link(base, "subsystem");
    }
    if let Some(parent) = base.tree_parent.get() {
        parent.with_edit(|parent| {
            let _ = parent.detach_child(base.name());
        });
        if base.parent().is_none() {
            let registry = crate::registry();
            registry
                .virtual_glue_dirs
                .drop_if_empty(subsystem.name(), registry.virtual_dir.as_ref());
        }
    }
}

const DEV_ATTR: Attr<dyn AnyDevice> = Attr::ro("dev", |dev, writer| {
    let devnum = dev.base().devnum().ok_or(Error::InvalidValue)?;
    writeln!(writer, "{devnum}")?;
    Ok(())
});
