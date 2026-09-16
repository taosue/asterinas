// SPDX-License-Identifier: MPL-2.0

//! Devices in a class.

use alloc::{
    sync::{Arc, Weak},
    vec::Vec,
};
use core::ops::Deref;

use aster_systree::{SysAttrSet, SysBranchNode, SysNode, SysNodeId, SysNodeType, SysObj, SysPerms};
use ostd::mm::{VmReader, VmWriter};

use super::{AnyDevice, DeviceBase, DeviceBuilder, DeviceInternals, Subsystem};
use crate::{
    DevNode, SysStr,
    attr::{Attr, TyErasedAttr},
    class::{Class, ClassHandle},
};

/// A device in class `C`: the interface user space sees.
///
/// Dereferences to the class-specific payload `C::Device`.
pub struct ClassDevice<C: Class> {
    base: DeviceBase,
    class: Arc<ClassHandle<C>>,
    payload: C::Device,
    attrs: &'static [Attr<Self>],
    weak: Weak<Self>,
}

/// Builds a [`ClassDevice`].
pub type ClassDeviceBuilder<C> =
    DeviceBuilder<Arc<ClassHandle<C>>, <C as Class>::Device, ClassDevice<C>>;

impl<C: Class> core::fmt::Debug for ClassDevice<C> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ClassDevice")
            .field("class", &C::NAME)
            .field("base", &self.base)
            .finish_non_exhaustive()
    }
}

impl<C: Class> Deref for ClassDevice<C> {
    type Target = C::Device;

    fn deref(&self) -> &C::Device {
        &self.payload
    }
}

impl<C: Class> ClassDevice<C> {
    /// Starts building a device in `class`.
    pub fn builder(
        class: &Arc<ClassHandle<C>>,
        name: impl Into<SysStr>,
        payload: C::Device,
    ) -> ClassDeviceBuilder<C> {
        DeviceBuilder::new(class.clone(), name.into(), payload)
    }

    /// Returns the class this device is in.
    pub fn class(&self) -> &Arc<ClassHandle<C>> {
        &self.class
    }

    /// Returns the class-specific payload.
    pub fn payload(&self) -> &C::Device {
        &self.payload
    }

    /// Returns a strong reference to this device.
    pub(crate) fn this(&self) -> Arc<Self> {
        self.weak
            .upgrade()
            .expect("a device is only reachable through an `Arc`")
    }
}

impl<C: Class> ClassDeviceBuilder<C> {
    /// Builds the device. It is not registered until
    /// [`add`](super::add) is called.
    ///
    /// # Panics
    ///
    /// Panics if the name is empty, `.` or `..`, or contains `/` or `NUL`.
    pub fn build(self) -> Arc<ClassDevice<C>> {
        Arc::new_cyclic(|weak: &Weak<ClassDevice<C>>| {
            let weak_self: Weak<dyn AnyDevice> = weak.clone();
            ClassDevice {
                base: DeviceBase::new(self.name, self.parent, self.devnum, weak_self),
                class: self.handle,
                payload: self.payload,
                attrs: self.attrs,
                weak: weak.clone(),
            }
        })
    }
}

impl<C: Class> AnyDevice for ClassDevice<C> {
    fn base(&self) -> &DeviceBase {
        &self.base
    }

    fn subsystem(&self) -> Subsystem {
        Subsystem::new(self.class.clone())
    }
}

impl<C: Class> DeviceInternals for ClassDevice<C> {
    fn devnode_override(&self) -> Option<DevNode> {
        self.class.class().devnode(self)
    }

    fn attributes(&self) -> Vec<TyErasedAttr> {
        let mut attrs = TyErasedAttr::from_typed_slice(self.class.class().dev_attrs());
        attrs.extend(TyErasedAttr::from_typed_slice(self.attrs));
        attrs
    }
}

impl<C: Class> SysObj for ClassDevice<C> {
    fn as_any(&self) -> &dyn core::any::Any {
        self
    }

    fn cast_to_node(&self) -> Option<Arc<dyn SysNode>> {
        self.base()
            .weak_self
            .upgrade()
            .map(|d| d as Arc<dyn SysNode>)
    }

    fn cast_to_branch(&self) -> Option<Arc<dyn SysBranchNode>> {
        self.base()
            .weak_self
            .upgrade()
            .map(|d| d as Arc<dyn SysBranchNode>)
    }

    fn id(&self) -> &SysNodeId {
        &self.base().id
    }

    fn type_(&self) -> SysNodeType {
        SysNodeType::Branch
    }

    fn name(&self) -> &SysStr {
        &self.base().name
    }

    fn init_parent(&self, parent: Weak<dyn SysBranchNode>) {
        self.base().sys_parent.call_once(|| parent);
    }

    fn parent(&self) -> Option<Arc<dyn SysBranchNode>> {
        self.base().sys_parent.get().and_then(|w| w.upgrade())
    }
}

impl<C: Class> SysNode for ClassDevice<C> {
    fn node_attrs(&self) -> Arc<SysAttrSet> {
        self.base().attrs.set()
    }

    fn is_attr_absent(&self, _name: &str) -> bool {
        !self.base().is_added()
    }

    fn read_attr(&self, name: &str, writer: &mut VmWriter) -> aster_systree::Result<usize> {
        self.read_attr_at(name, 0, writer)
    }

    fn write_attr(&self, name: &str, reader: &mut VmReader) -> aster_systree::Result<usize> {
        if !self.base().is_added() {
            return Err(aster_systree::Error::IsDead);
        }
        self.base().attrs.store(self, name, reader)
    }

    fn read_attr_at(
        &self,
        name: &str,
        offset: usize,
        writer: &mut VmWriter,
    ) -> aster_systree::Result<usize> {
        if !self.base().is_added() {
            return Err(aster_systree::Error::IsDead);
        }
        self.base().attrs.show(self, name, offset, writer)
    }

    fn write_attr_at(
        &self,
        name: &str,
        _offset: usize,
        reader: &mut VmReader,
    ) -> aster_systree::Result<usize> {
        self.write_attr(name, reader)
    }

    fn perms(&self) -> SysPerms {
        SysPerms::DEFAULT_RW_PERMS
    }
}

impl<C: Class> SysBranchNode for ClassDevice<C> {
    fn visit_child_with(&self, name: &str, f: &mut dyn FnMut(Option<&Arc<dyn SysObj>>)) {
        let children = self.base().children.read();
        f(children.get(name))
    }

    fn visit_children_with(
        &self,
        min_id: u64,
        f: &mut dyn for<'a> FnMut(&'a Arc<dyn SysObj>) -> Option<()>,
    ) {
        let children = self.base().children.read();
        for child in children.values() {
            if child.id().as_u64() < min_id {
                continue;
            }
            if f(child).is_none() {
                break;
            }
        }
    }

    fn child(&self, name: &str) -> Option<Arc<dyn SysObj>> {
        self.base().children.read().get(name).cloned()
    }
}

impl<C: Class> crate::node::Sealed for ClassDevice<C> {}

impl<C: Class> crate::Container for ClassDevice<C> {}
