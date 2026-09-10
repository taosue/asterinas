// SPDX-License-Identifier: MPL-2.0

//! Class views and class glue directories.

use alloc::{format, sync::Arc};

use aster_systree::{
    BranchNodeFields, SymlinkNodeFields, SysAttrSet, SysBranchNode, SysObj, SysPerms, SysStr,
    inherit_sys_branch_node, inherit_sys_symlink_node,
};
use spin::Once;

use crate::AnyDevice;

/// A class-based view of devices.
pub trait Class: SysBranchNode {
    /// Whether devices in this class use a class directory and registration.
    const HAS_CLASS: bool = true;

    /// Returns the class name.
    fn name() -> &'static str;

    /// The device type represented by this class.
    type Device: AnyDevice + ?Sized;

    /// Registers a device in this class's subsystem state.
    fn register(device: Arc<Self::Device>) -> aster_systree::Result<()>;
}

/// Converts an accepted device into a class's device interface.
pub trait ClassFor<D>: Class {
    fn into_class_device(device: Arc<D>) -> Arc<Self::Device>;
}

/// The class marker for devices without a class.
#[derive(Debug)]
pub struct NoClass {
    fields: BranchNodeFields<dyn SysObj, Self>,
}

inherit_sys_branch_node!(NoClass, fields, {
    fn perms(&self) -> SysPerms {
        SysPerms::DEFAULT_RO_PERMS
    }
});

impl Class for NoClass {
    const HAS_CLASS: bool = false;

    type Device = dyn AnyDevice;

    fn name() -> &'static str {
        "no_class"
    }

    fn register(_device: Arc<Self::Device>) -> aster_systree::Result<()> {
        Ok(())
    }
}

impl<D: AnyDevice> ClassFor<D> for NoClass {
    fn into_class_device(device: Arc<D>) -> Arc<Self::Device> {
        device
    }
}

/// Registers a class below `/sys/class`.
pub fn register_class<T: Class>(class: Arc<T>) -> aster_systree::Result<Arc<T>> {
    CLASS_ROOT.get().unwrap().fields.add_child(class.clone())?;
    Ok(class)
}

/// Generates device registration methods for a topology parent.
///
/// The generated method owns the class-glue lookup and creation logic while
/// keeping the registration entry point on the concrete topology parent.
#[macro_export]
macro_rules! impl_device_parent {
    ($parent:ty, $field:ident $(, $vis:vis)?) => {
        impl $parent {
            fn lookup_or_create_dir<T: $crate::Class>(
                &self,
            ) -> ::aster_systree::Result<
                ::alloc::sync::Arc<$crate::ClassGlueDir<T>>,
            > {
                if let Some(child) = self.$field.child(<T as $crate::Class>::name()) {
                    let dir = child
                        .as_any()
                        .downcast_ref::<$crate::ClassGlueDir<T>>()
                        .ok_or(::aster_systree::Error::InvalidOperation)?;
                    return Ok(dir.this());
                }

                let dir = $crate::ClassGlueDir::<T>::new();
                match self.$field.add_child(dir.clone()) {
                    Ok(()) => Ok(dir),
                    Err(::aster_systree::Error::AlreadyExists) => {
                        self.lookup_or_create_dir::<T>()
                    }
                    Err(error) => Err(error),
                }
            }

            $($vis)? fn add_device<T: $crate::AnyDevice + $crate::IsChild<Self>>(
                &self,
                device: ::alloc::sync::Arc<T>,
            ) -> ::aster_systree::Result<()> {
                if !<T::Class as $crate::Class>::HAS_CLASS {
                    return self.$field.add_child(device);
                }

                let dir = if <<Self as $crate::AnyDevice>::Class as $crate::Class>::HAS_CLASS {
                    self.$field.add_child(device.clone())?;
                    None
                } else {
                    Some(self.lookup_or_create_dir::<T::Class>()?)
                };
                let class_device = <T::Class as $crate::ClassFor<T>>::into_class_device(device);
                if let Some(dir) = dir {
                    dir.add_device(class_device.clone())?;
                }
                <T::Class as $crate::Class>::register(class_device)
            }
        }
    };
}

/// Initializes the `/sys/class` root.
pub(super) fn init() {
    CLASS_ROOT.call_once(|| {
        let root = ClassesRoot::new();
        aster_systree::primary_tree()
            .root()
            .add_child(root.clone())
            .unwrap();
        root
    });
}

/// A class glue directory in the device topology.
#[derive(Debug)]
pub struct ClassGlueDir<T: Class> {
    fields: BranchNodeFields<T::Device, Self>,
}

impl<T: Class> ClassGlueDir<T> {
    /// Creates the glue directory for `T`.
    pub fn new() -> Arc<Self> {
        Arc::new_cyclic(|weak_self| Self {
            fields: BranchNodeFields::new(
                SysStr::from(<T as Class>::name()),
                SysAttrSet::new_empty(),
                weak_self.clone(),
            ),
        })
    }

    /// Adds a device belonging to this class below the glue directory.
    pub fn add_device(&self, device: Arc<T::Device>) -> aster_systree::Result<()> {
        self.fields.add_child(device)
    }

    /// Returns an owned reference to this directory.
    pub fn this(&self) -> Arc<Self> {
        self.fields.weak_self().upgrade().unwrap()
    }
}

inherit_sys_branch_node!(ClassGlueDir<T: Class>, fields, {
    fn perms(&self) -> SysPerms {
        SysPerms::DEFAULT_RO_PERMS
    }
});

/// A symlink used by a class view to point at a device in `/sys/devices`.
#[derive(Debug)]
pub struct ClassDeviceLink {
    fields: SymlinkNodeFields<Self>,
}

impl ClassDeviceLink {
    /// Creates a class-view link with `name` pointing to a device at `path`.
    pub fn new(name: SysStr, path: &str) -> Arc<Self> {
        let target_path = format!("../..{}", path);
        Arc::new_cyclic(|weak_self| Self {
            fields: SymlinkNodeFields::new(name, target_path, weak_self.clone()),
        })
    }
}

inherit_sys_symlink_node!(ClassDeviceLink, fields, {});

#[derive(Debug)]
struct ClassesRoot {
    fields: BranchNodeFields<dyn SysObj, Self>,
}

impl ClassesRoot {
    fn new() -> Arc<Self> {
        Arc::new_cyclic(|weak_self| Self {
            fields: BranchNodeFields::new(
                SysStr::from("class"),
                SysAttrSet::new_empty(),
                weak_self.clone(),
            ),
        })
    }
}

inherit_sys_branch_node!(ClassesRoot, fields, {
    fn perms(&self) -> SysPerms {
        SysPerms::DEFAULT_RO_PERMS
    }
});

static CLASS_ROOT: Once<Arc<ClassesRoot>> = Once::new();
