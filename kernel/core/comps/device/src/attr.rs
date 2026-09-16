// SPDX-License-Identifier: MPL-2.0

//! Attributes: the files inside a device's sysfs directory.
//!
//! [`Attr`] binds static text attributes to a concrete device type. Registration
//! merges class and instance declarations into a mutable table. Each change
//! publishes an immutable attribute-set snapshot for sysfs. Erased callbacks
//! let sysfs access devices without knowing their payload types.

use alloc::{
    collections::{BTreeMap, BTreeSet},
    string::String,
    sync::Arc,
    vec,
    vec::Vec,
};
use core::fmt::{self, Write};

use aster_systree::{MAX_ATTR_SIZE, SysAttrSet, SysAttrSetBuilder, SysPerms};
use aster_util::printer::VmPrinter;
use ostd::{
    mm::{FallibleVmRead, VmReader, VmWriter},
    sync::RwMutex,
};

use crate::{Error, Result, SysStr, device::AnyDevice};

/// Produces the text of an attribute.
pub type ShowFn<D> = fn(&D, &mut dyn Write) -> Result<()>;

/// Consumes the text written to an attribute.
pub type StoreFn<D> = fn(&D, &str) -> Result<()>;

/// A statically declared attribute of devices of type `D`.
pub struct Attr<D: ?Sized> {
    name: &'static str,
    perms: SysPerms,
    show_fn: Option<ShowFn<D>>,
    store_fn: Option<StoreFn<D>>,
}

impl<D: ?Sized> Attr<D> {
    /// A read-only attribute.
    pub const fn ro(name: &'static str, show_fn: ShowFn<D>) -> Self {
        Self {
            name,
            perms: SysPerms::DEFAULT_RO_ATTR_PERMS,
            show_fn: Some(show_fn),
            store_fn: None,
        }
    }

    /// A read-write attribute.
    pub const fn rw(name: &'static str, show_fn: ShowFn<D>, store_fn: StoreFn<D>) -> Self {
        Self {
            name,
            perms: SysPerms::DEFAULT_RW_ATTR_PERMS,
            show_fn: Some(show_fn),
            store_fn: Some(store_fn),
        }
    }

    /// A write-only attribute.
    pub const fn wo(name: &'static str, store_fn: StoreFn<D>) -> Self {
        Self {
            name,
            perms: SysPerms::OWNER_W,
            show_fn: None,
            store_fn: Some(store_fn),
        }
    }

    /// Returns the name.
    pub fn name(&self) -> &'static str {
        self.name
    }

    /// Returns the permissions.
    pub fn perms(&self) -> SysPerms {
        self.perms
    }
}

impl<D: ?Sized> Clone for Attr<D> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<D: ?Sized> Copy for Attr<D> {}

impl<D: ?Sized> fmt::Debug for Attr<D> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Attr")
            .field("name", &self.name)
            .field("perms", &self.perms)
            .finish()
    }
}

type TyErasedShowFn = Arc<dyn Fn(&dyn AnyDevice, &mut dyn Write) -> Result<()> + Send + Sync>;
type TyErasedStoreFn = Arc<dyn Fn(&dyn AnyDevice, &str) -> Result<()> + Send + Sync>;

/// An attribute whose callbacks take `&dyn AnyDevice`.
#[derive(Clone)]
pub(crate) struct TyErasedAttr {
    name: &'static str,
    perms: SysPerms,
    show_fn: Option<TyErasedShowFn>,
    store_fn: Option<TyErasedStoreFn>,
}

impl fmt::Debug for TyErasedAttr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TyErasedAttr")
            .field("name", &self.name)
            .field("perms", &self.perms)
            .finish()
    }
}

impl TyErasedAttr {
    /// Erases an attribute declared for the concrete device type `D`.
    pub(crate) fn from_typed<D: AnyDevice>(attr: &Attr<D>) -> Self {
        let show_fn = attr.show_fn.map(|show_fn| -> TyErasedShowFn {
            Arc::new(move |dev: &dyn AnyDevice, w: &mut dyn Write| {
                let dev = dev.as_any().downcast_ref::<D>().unwrap();
                show_fn(dev, w)
            })
        });
        let store_fn = attr.store_fn.map(|store_fn| -> TyErasedStoreFn {
            Arc::new(move |dev: &dyn AnyDevice, s: &str| {
                let dev = dev.as_any().downcast_ref::<D>().unwrap();
                store_fn(dev, s)
            })
        });
        Self {
            name: attr.name,
            perms: attr.perms,
            show_fn,
            store_fn,
        }
    }

    /// Wraps an attribute that already takes the erased device view.
    pub(crate) fn from_dyn(attr: &Attr<dyn AnyDevice>) -> Self {
        Self {
            name: attr.name,
            perms: attr.perms,
            show_fn: attr
                .show_fn
                .map(|show_fn| -> TyErasedShowFn { Arc::new(show_fn) }),
            store_fn: attr
                .store_fn
                .map(|store_fn| -> TyErasedStoreFn { Arc::new(store_fn) }),
        }
    }

    /// Erases a whole slice of typed attributes.
    pub(crate) fn from_typed_slice<D: AnyDevice>(attrs: &[Attr<D>]) -> Vec<Self> {
        attrs.iter().map(Self::from_typed).collect()
    }
}

/// The attributes of one device: the `SysTree` attribute set that sysfs
/// lists, plus the callbacks behind each entry.
///
/// A device's attributes arrive in layers and a driver's go away again, so the
/// table changes over the device's life. What it publishes to `SysTree` does
/// not: a [`SysAttrSet`] is immutable, so every change builds a whole new set
/// from the old one and swaps it in. [`SysAttrSetBuilder::from_set`] carries
/// the surviving attributes across with the IDs they had, which is what keeps a
/// file's inode number stable when a driver binds beside it.
///
/// The set and the callbacks live under one lock, so the two never disagree
/// about which attributes exist.
#[derive(Debug)]
pub(crate) struct AttrTable {
    inner: RwMutex<TableInner>,
}

#[derive(Debug)]
struct TableInner {
    /// What sysfs lists. Replaced wholesale, never edited.
    set: Arc<SysAttrSet>,
    /// What serves a read or a write of each listed attribute.
    ops: BTreeMap<SysStr, TyErasedAttr>,
}

impl AttrTable {
    pub(crate) fn new() -> Self {
        Self {
            inner: RwMutex::new(TableInner {
                set: SysAttrSet::empty().clone(),
                ops: BTreeMap::new(),
            }),
        }
    }

    /// The attribute set, in the form sysfs consumes.
    pub(crate) fn set(&self) -> Arc<SysAttrSet> {
        self.inner.read().set.clone()
    }

    /// Adds attributes. Fails if any name is already present or is repeated
    /// within `attrs`, or if the ID space is exhausted; in every case none of
    /// the attributes is added.
    ///
    /// Repetition has to be caught here rather than left to the set builder,
    /// which treats a repeated name as a no-op: registration hands the layers
    /// of one device over as a single batch, so without this check a class that
    /// declared an attribute named `dev` would quietly take over the core's
    /// callback while sysfs went on showing the core's entry.
    pub(crate) fn add(&self, attrs: Vec<TyErasedAttr>) -> Result<()> {
        let mut inner = self.inner.write();
        let mut seen = BTreeSet::new();
        if attrs
            .iter()
            .any(|attr| inner.ops.contains_key(attr.name) || !seen.insert(attr.name))
        {
            return Err(Error::NameConflict);
        }

        let mut builder = SysAttrSetBuilder::from_set(&inner.set);
        for attr in &attrs {
            builder.add(SysStr::from(attr.name), attr.perms);
        }
        // The new set is built to one side, so an exhausted ID space is
        // reported here with the table still exactly as it was.
        inner.set = Arc::new(builder.build()?);
        for attr in attrs {
            inner.ops.insert(SysStr::from(attr.name), attr);
        }
        Ok(())
    }

    /// Removes attributes by name. Names that are absent are ignored.
    pub(crate) fn remove(&self, names: &[&'static str]) {
        let mut inner = self.inner.write();
        let mut builder = SysAttrSetBuilder::from_set(&inner.set);
        for name in names {
            inner.ops.remove(*name);
            builder.remove(name);
        }
        let set = builder
            .build()
            .expect("a builder that only removes attributes cannot fail");
        inner.set = Arc::new(set);
    }

    /// Serves a read of an attribute, honoring `offset` the way `VmPrinter`
    /// does: the whole text is produced and the first `offset` bytes skipped.
    /// Runs the `show` callback of `name`.
    ///
    /// The callback runs after the lock is released, so that no attribute
    /// callback holds a lock of the device model. The cost is that a call
    /// already in flight can finish after its attribute has been removed; it
    /// stays safe because the caller holds the device alive. Linux instead
    /// drains active references in `kernfs_drain`.
    pub(crate) fn show(
        &self,
        dev: &dyn AnyDevice,
        name: &str,
        offset: usize,
        writer: &mut VmWriter,
    ) -> aster_systree::Result<usize> {
        let show_fn = {
            let inner = self.inner.read();
            let attr = inner.ops.get(name).ok_or(aster_systree::Error::NotFound)?;
            attr.show_fn
                .clone()
                .ok_or(aster_systree::Error::PermissionDenied)?
        };
        let mut printer = VmPrinter::new_skip(writer, offset);
        show_fn(dev, &mut printer).map_err(aster_systree::Error::from)?;
        Ok(printer.bytes_written())
    }

    /// Serves a write of an attribute. The written bytes are taken as text
    /// with any trailing newline removed.
    pub(crate) fn store(
        &self,
        dev: &dyn AnyDevice,
        name: &str,
        reader: &mut VmReader,
    ) -> aster_systree::Result<usize> {
        let store_fn = {
            let inner = self.inner.read();
            let attr = inner.ops.get(name).ok_or(aster_systree::Error::NotFound)?;
            attr.store_fn
                .clone()
                .ok_or(aster_systree::Error::PermissionDenied)?
        };
        let (text, len) = read_text(reader)?;
        store_fn(dev, &text).map_err(aster_systree::Error::from)?;
        Ok(len)
    }
}

/// Reads at most `MAX_ATTR_SIZE` bytes from user space as text.
pub(crate) fn read_text(reader: &mut VmReader) -> aster_systree::Result<(String, usize)> {
    let mut buf = vec![0u8; MAX_ATTR_SIZE];
    let mut writer = VmWriter::from(buf.as_mut_slice());
    let len = reader
        .read_fallible(&mut writer)
        .map_err(|_| aster_systree::Error::PageFault)?;
    let text = core::str::from_utf8(&buf[..len])
        .map_err(|_| aster_systree::Error::InvalidOperation)?
        .trim_end_matches('\n');
    Ok((String::from(text), len))
}

#[cfg(ktest)]
mod test;
