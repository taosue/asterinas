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

mod file;

use aster_device::{AnyDevice, Class, ClassDevice, ClassHandle, DevNode, DevNum};
use device_id::{DeviceId, MajorId, MinorId};
use file::MemFile;
pub(crate) use file::{getrandom, geturandom};
use spin::Once;

use super::{
    Device, DeviceType,
    registry::char::{self, MajorIdOwner},
};
use crate::{
    fs::{
        devtmpfs::DevtmpfsNodeMeta,
        file::{PerOpenFileOps, mkmod},
    },
    prelude::*,
};

struct MemClass;

type MemDevice = ClassDevice<MemClass>;

impl Class for MemClass {
    const NAME: &'static str = "mem";
    type Device = MemFile;

    fn devnode(&self, dev: &MemDevice) -> Option<DevNode> {
        // Preserve the memory-device permissions while leaving naming to the
        // model's default policy. Linux uses the same per-minor modes:
        // <https://elixir.bootlin.com/linux/v6.18/source/drivers/char/mem.c#L690>.
        let mode = match dev.payload() {
            MemFile::Full | MemFile::Null | MemFile::Random | MemFile::Urandom | MemFile::Zero => {
                mkmod!(a+rw)
            }
            MemFile::Kmsg => mkmod!(a+r, u+w),
            _ => return None,
        };
        Some(DevNode {
            path: None,
            mode: Some(mode.bits()),
        })
    }
}

impl Device for MemDevice {
    fn type_(&self) -> DeviceType {
        DeviceType::Char
    }

    fn id(&self) -> DeviceId {
        self.base()
            .devnum()
            .expect("memory devices have a device number")
            .id()
    }

    fn devtmpfs_meta(&self) -> Option<DevtmpfsNodeMeta> {
        // The device model owns this node's creation and removal.
        None
    }

    fn open(&self) -> Result<Box<dyn PerOpenFileOps>> {
        Ok(Box::new(*self.payload()))
    }
}

static MEM_MAJOR: Once<MajorIdOwner> = Once::new();
static MEM_CLASS: Once<Arc<ClassHandle<MemClass>>> = Once::new();

pub(super) fn init_in_first_kthread() {
    MEM_MAJOR.call_once(|| char::acquire_major(MajorId::new(1)).unwrap());
    MEM_CLASS.call_once(|| aster_device::register_class(MemClass).unwrap());

    for file in [
        MemFile::Full,
        MemFile::Null,
        MemFile::Random,
        MemFile::Urandom,
        MemFile::Zero,
    ] {
        add_device(file).unwrap();
    }
}

fn add_device(file: MemFile) -> Result<()> {
    let class = MEM_CLASS.get().unwrap();
    let id = DeviceId::new(MEM_MAJOR.get().unwrap().get(), MinorId::new(file.minor()));
    let device = ClassDevice::builder(class, file.name(), file)
        .devnum(DevNum::char(id))
        .build();

    // Publish the open backend before the model creates /dev/<name>. The
    // payload is already complete, so even an existing mknod can open it.
    char::register(device.clone())?;
    let mut pending = PendingCharRegistration { id: Some(id) };
    aster_device::add(&device)?;
    pending.id = None;
    Ok(())
}

/// Unregisters the open backend if model registration fails.
struct PendingCharRegistration {
    id: Option<DeviceId>,
}

impl Drop for PendingCharRegistration {
    fn drop(&mut self) {
        if let Some(id) = self.id
            && let Err(error) = char::unregister(id)
        {
            warn!("failed to roll back memory device {:?}: {:?}", id, error);
        }
    }
}

#[cfg(ktest)]
mod test;
