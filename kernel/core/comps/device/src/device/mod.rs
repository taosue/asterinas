// SPDX-License-Identifier: MPL-2.0

//! Typed class devices and the erased view used by registration and sysfs.

mod class_device;
mod registration;

use alloc::{
    collections::BTreeMap,
    string::String,
    sync::{Arc, Weak},
    vec::Vec,
};

use aster_systree::{SysBranchNode, SysObj};
use ostd::sync::{Mutex, RwMutex};
use spin::Once;

pub use self::{
    class_device::{ClassDevice, ClassDeviceBuilder},
    registration::{DeviceBuilder, add, remove},
};
use crate::{
    Error, Result, SysStr,
    attr::{AttrTable, TyErasedAttr},
    node::{Dir, SysTreeEdit},
};

/// The subsystem that owns a device.
#[derive(Clone)]
pub struct Subsystem {
    ops: Arc<dyn SubsystemOps>,
}

impl core::fmt::Debug for Subsystem {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Subsystem")
            .field("name", &self.name())
            .finish()
    }
}

impl Subsystem {
    pub(super) fn new(ops: Arc<dyn SubsystemOps>) -> Self {
        Self { ops }
    }

    /// Returns the subsystem name.
    pub fn name(&self) -> &'static str {
        self.ops.name()
    }
}

/// The subsystem operations needed by the shared registration sequence.
///
/// Keeping this view independent of `Class` allows bus support to reuse the
/// same directory and membership lifecycle later.
pub(crate) trait SubsystemOps: Send + Sync + 'static {
    fn name(&self) -> &'static str;
    fn dir(&self) -> Arc<Dir>;
    fn index_dir(&self) -> Arc<Dir>;
    fn on_added(&self, dev: &Arc<dyn AnyDevice>);
    fn on_removed(&self, dev: &Arc<dyn AnyDevice>);
}

/// The life cycle of a device. Registration and removal each happen once.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    /// Built, not yet in the tree.
    Initialized,
    /// Registered.
    Added,
    /// Removed; the object may still be referenced but is dead.
    Removed,
}

/// The symlinks a device's registration has created so far.
#[derive(Default)]
struct Links {
    subsystem: bool,
    device: bool,
    index: bool,
}

/// Where a device's directory was placed: a plain directory (a root, a glue
/// directory) or another device's directory.
enum TreeParent {
    Dir(Weak<Dir>),
    Device(Weak<dyn AnyDevice>),
}

impl TreeParent {
    /// Runs `f` with the crate-private editing view of the parent, if the
    /// parent is still alive.
    fn with_edit<R>(&self, f: impl FnOnce(&dyn SysTreeEdit) -> R) -> Option<R> {
        match self {
            TreeParent::Dir(dir) => dir.upgrade().map(|dir| f(dir.as_ref())),
            TreeParent::Device(dev) => dev.upgrade().map(|dev| f(dev.base())),
        }
    }
}

/// The part of a device that the registration sequence works on.
pub struct DeviceBase {
    id: aster_systree::SysNodeId,
    name: SysStr,
    /// The branch node this device's directory lives in, as `systree` sees
    /// it. Set when the device is attached.
    sys_parent: Once<Weak<dyn SysBranchNode>>,
    /// The same parent, as the registration sequence edits it.
    tree_parent: Once<TreeParent>,
    /// The entries of the device's directory: symlinks, glue directories, and
    /// child devices of any kind.
    children: RwMutex<BTreeMap<SysStr, Arc<dyn SysObj>>>,
    attrs: AttrTable,
    weak_self: Weak<dyn AnyDevice>,
    /// The device this one is reached through, if any.
    parent: Option<Arc<dyn AnyDevice>>,
    state: Mutex<State>,
    /// Which of the symlinks that `add` may create exist, so that `detach`
    /// removes only links this device made.
    links: Mutex<Links>,
    /// Devices whose parent is this one.
    child_devices: Mutex<Vec<Weak<dyn AnyDevice>>>,
}

impl core::fmt::Debug for DeviceBase {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("DeviceBase")
            .field("name", &self.name)
            .field("state", &*self.state.lock())
            .finish_non_exhaustive()
    }
}

impl DeviceBase {
    fn new(
        name: SysStr,
        parent: Option<Arc<dyn AnyDevice>>,
        weak_self: Weak<dyn AnyDevice>,
    ) -> Self {
        assert!(
            !aster_systree::is_invalid_name(&name),
            "invalid SysTree node name"
        );
        Self {
            id: aster_systree::SysNodeId::new(),
            name,
            sys_parent: Once::new(),
            tree_parent: Once::new(),
            children: RwMutex::new(BTreeMap::new()),
            attrs: AttrTable::new(),
            weak_self,
            parent,
            state: Mutex::new(State::Initialized),
            links: Mutex::new(Links::default()),
            child_devices: Mutex::new(Vec::new()),
        }
    }

    /// Returns the device name, which is also its directory name.
    pub fn name(&self) -> &SysStr {
        &self.name
    }

    /// Returns the parent device, if any.
    pub fn parent(&self) -> Option<&Arc<dyn AnyDevice>> {
        self.parent.as_ref()
    }

    /// Returns whether registration has completed and the device is still present.
    pub fn is_added(&self) -> bool {
        *self.state.lock() == State::Added
    }

    /// Returns the registered child devices, skipping any that have been
    /// dropped.
    pub fn child_devices(&self) -> Vec<Arc<dyn AnyDevice>> {
        self.child_devices
            .lock()
            .iter()
            .filter_map(|w| w.upgrade())
            .collect()
    }
}

impl SysTreeEdit for DeviceBase {
    fn attach_child(&self, child: Arc<dyn SysObj>) -> Result<()> {
        let mut children = self.children.write();
        let name = child.name();
        if aster_systree::is_invalid_name(name) {
            return Err(Error::InvalidName);
        }
        if children.contains_key(name) || self.attrs.set().contains(name) {
            return Err(Error::NameConflict);
        }
        let weak: Weak<dyn SysBranchNode> = self.weak_self.clone();
        child.init_parent(weak);
        children.insert(name.clone(), child);
        Ok(())
    }

    fn detach_child(&self, name: &str) -> Result<Arc<dyn SysObj>> {
        self.children.write().remove(name).ok_or(Error::NotFound)
    }

    fn has_children(&self) -> bool {
        !self.children.read().is_empty()
    }

    /// Returns the device's path, computed as [`SysObj::path`] computes it.
    fn tree_path(&self) -> String {
        let Some(parent) = self.sys_parent.get().and_then(|w| w.upgrade()) else {
            return String::from(self.name.as_ref());
        };
        let mut path = String::from(parent.path().as_ref());
        if !parent.is_root() {
            path.push('/');
        }
        path.push_str(&self.name);
        path
    }
}

/// The private callbacks used by registration.
pub(crate) trait DeviceInternals {
    fn attributes(&self) -> Vec<TyErasedAttr>;
}

/// The erased device view used for registration, parent links, and sysfs.
///
/// Implementations are provided by this crate. Class callbacks receive the
/// concrete [`ClassDevice`] and its typed payload instead.
#[expect(private_bounds, reason = "registration callbacks are crate-private")]
pub trait AnyDevice: crate::Container + DeviceInternals {
    /// Returns the device's identity and registration state.
    fn base(&self) -> &DeviceBase;

    /// Returns the subsystem that owns this device.
    fn subsystem(&self) -> Subsystem;

    /// Returns a strong reference to the device.
    fn to_arc(&self) -> Arc<dyn AnyDevice> {
        self.base()
            .weak_self
            .upgrade()
            .expect("devices are owned through Arc")
    }
}
