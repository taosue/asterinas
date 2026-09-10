// SPDX-License-Identifier: MPL-2.0

//! Memory devices.
//!
//! Character device with major number 1. The minor numbers are mapped as follows:
//! - 1 = /dev/mem      Physical memory access
//! - 2 = /dev/kmem     OBSOLETE - replaced by /proc/kcore
//! - 3 = /dev/null     Null device
//! - 4 = /dev/port     I/O port access
//! - 5 = /dev/zero     Null byte source
//! - 6 = /dev/core     OBSOLETE - replaced by /proc/kcore
//! - 7 = /dev/full     Returns ENOSPC on write
//! - 8 = /dev/random   Nondeterministic random number gen.
//! - 9 = /dev/urandom  Faster, less secure random number gen.
//! - 10 = /dev/aio     Asynchronous I/O notification interface
//! - 11 = /dev/kmsg    Writes to this come out as printk's, reads export the buffered printk records.
//! - 12 = /dev/oldmem  OBSOLETE - replaced by /proc/vmcore
//!
//! See <https://www.kernel.org/doc/Documentation/admin-guide/devices.txt>.

mod class;
mod file;

use aster_device::{AnyDevice, IsChild};
use aster_systree::{
    BranchNodeFields, Error as SysError, Result as SysResult, SysAttrSetBuilder, SysObj, SysPerms,
    SysStr, inherit_sys_branch_node,
};
use aster_util::printer::VmPrinter;
use class::MemClass;
use device_id::{DeviceId, MajorId, MinorId};
use file::MemFile;
pub(crate) use file::{getrandom, geturandom};
use spin::Once;

use super::{
    DevNode, DeviceType,
    registry::char::{self, MajorIdOwner},
    virtual_bus::{self, VirtualBusDevice},
};
use crate::{
    fs::{
        devtmpfs::DevtmpfsNodeMeta,
        file::{PerOpenFileOps, mkmod},
    },
    prelude::*,
};

/// A memory device.
#[derive(Debug)]
pub(crate) struct MemDevice {
    fields: BranchNodeFields<dyn SysObj, Self>,
    id: DeviceId,
    file: MemFile,
}

impl MemDevice {
    fn new(file: MemFile) -> Result<Arc<Self>> {
        let major = MEM_MAJOR.get().unwrap().get();
        let minor = MinorId::new(file.minor());

        let mut builder = SysAttrSetBuilder::new();
        builder.add(SysStr::from("dev"), SysPerms::DEFAULT_RO_ATTR_PERMS);
        let attrs = builder.build()?;

        let device = Arc::new_cyclic(|weak_self| Self {
            fields: BranchNodeFields::new(SysStr::from(file.name()), attrs, weak_self.clone()),
            id: DeviceId::new(major, minor),
            file,
        });

        virtual_bus::register_device(device.clone())?;
        char::register(device.clone())?;
        aster_device::register_dev_node(device.path().as_ref(), DeviceType::Char, device.id)?;

        Ok(device)
    }
}

impl AnyDevice for MemDevice {
    type Class = MemClass;
}

impl IsChild<VirtualBusDevice> for MemDevice {}

inherit_sys_branch_node!(MemDevice, fields, {
    fn read_attr_at(&self, name: &str, offset: usize, writer: &mut VmWriter) -> SysResult<usize> {
        match name {
            "dev" => {
                let mut printer = VmPrinter::new_skip(writer, offset);
                writeln!(
                    printer,
                    "{}:{}",
                    self.id.major().get(),
                    self.id.minor().get()
                )?;
                Ok(printer.bytes_written())
            }
            _ => Err(SysError::AttributeError),
        }
    }

    fn perms(&self) -> SysPerms {
        SysPerms::DEFAULT_RO_PERMS
    }
});

impl DevNode for MemDevice {
    fn type_(&self) -> DeviceType {
        DeviceType::Char
    }

    fn id(&self) -> DeviceId {
        self.id
    }

    fn devtmpfs_meta(&self) -> Option<DevtmpfsNodeMeta> {
        // Linux's memory-device table uses nonzero modes only for devices
        // that override devtmpfs's default `u+rw` permissions.
        // Reference: <https://elixir.bootlin.com/linux/v6.18/source/drivers/char/mem.c#L690>.
        // Reference: <https://elixir.bootlin.com/linux/v6.18/source/drivers/char/mem.c#L734>.
        Some(
            match self.file {
                MemFile::Full
                | MemFile::Null
                | MemFile::Random
                | MemFile::Urandom
                | MemFile::Zero => DevtmpfsNodeMeta::with_mode(self.file.name(), mkmod!(a+rw)),
                MemFile::Kmsg => DevtmpfsNodeMeta::with_mode(self.file.name(), mkmod!(a+r, u+w)),
                _ => DevtmpfsNodeMeta::new(self.file.name()),
            }
            .unwrap(),
        )
    }

    fn open(&self) -> Result<Box<dyn PerOpenFileOps>> {
        Ok(Box::new(self.file))
    }
}

static MEM_MAJOR: Once<MajorIdOwner> = Once::new();

pub(super) fn init_in_first_kthread() {
    MEM_MAJOR.call_once(|| char::acquire_major(MajorId::new(1)).unwrap());
    class::init_in_first_kthread();

    MemDevice::new(MemFile::Full).unwrap();
    MemDevice::new(MemFile::Null).unwrap();
    MemDevice::new(MemFile::Random).unwrap();
    MemDevice::new(MemFile::Urandom).unwrap();
    MemDevice::new(MemFile::Zero).unwrap();
}
