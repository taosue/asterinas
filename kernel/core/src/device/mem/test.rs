// SPDX-License-Identifier: MPL-2.0

use alloc::format;

use aster_device::ClassDevice;
use aster_systree::SysObj;
use device_id::{DeviceId, MajorId, MinorId};
use ostd::prelude::ktest;

use super::{MEM_CLASS, MemFile};
use crate::{
    device::{self, DeviceType, model, registry::char},
    fs::{
        devtmpfs,
        file::{AccessMode, InodeType, StatusFlags, mkmod},
    },
    prelude::*,
    util::random,
};

fn sysnode(path: &str) -> Option<Arc<dyn SysObj>> {
    let mut node: Arc<dyn SysObj> = aster_systree::primary_tree().root().clone();
    for name in path.split('/').filter(|name| !name.is_empty()) {
        node = node.cast_to_branch()?.child(name)?;
    }
    Some(node)
}

#[ktest]
fn memory_devices_open_and_registration_failures_clean_up() {
    let dev_root = devtmpfs::init_for_ktest();
    aster_device::init_for_ktest();
    model::install_hooks();
    random::init();
    super::init_in_first_kthread();

    let class = MEM_CLASS.get().unwrap();
    assert_eq!(class.devices().len(), 5);
    for (name, minor) in [
        ("null", 3),
        ("zero", 5),
        ("full", 7),
        ("random", 8),
        ("urandom", 9),
    ] {
        let id = DeviceId::new(MajorId::new(1), MinorId::new(minor));
        let inode = dev_root.lookup(name).unwrap();
        assert_eq!(inode.type_(), InodeType::CharDevice);
        let metadata = inode.metadata().unwrap();
        assert_eq!(metadata.self_dev_id, Some(id));
        assert_eq!(metadata.mode, mkmod!(a+rw));
        let file = inode
            .open(AccessMode::O_RDWR, StatusFlags::empty())
            .unwrap()
            .unwrap();
        let mut bytes = [0xff; 16];
        let mut writer = VmWriter::from(&mut bytes[..]).to_fallible();
        let read = file.read_at(0, &mut writer, StatusFlags::empty()).unwrap();
        assert_eq!(read, if name == "null" { 0 } else { 16 });
        if matches!(name, "zero" | "full") {
            assert_eq!(bytes, [0; 16]);
        }
        let mut reader = VmReader::from(&b"hello"[..]).to_fallible();
        let result = file.write_at(0, &mut reader, StatusFlags::empty());
        if name == "full" {
            assert_eq!(result.unwrap_err().error(), Errno::ENOSPC);
        } else {
            assert_eq!(result.unwrap(), 5);
        }

        let directory = sysnode(&format!("devices/virtual/mem/{name}"))
            .unwrap()
            .cast_to_node()
            .unwrap();
        assert_eq!(directory.show_attr("dev").unwrap(), format!("1:{minor}\n"));
        assert!(sysnode(&format!("dev/char/1:{minor}")).is_some());
        assert!(sysnode(&format!("class/mem/{name}")).is_some());
    }

    // A duplicate number must not disturb either registration of the original.
    assert_eq!(
        super::add_device(MemFile::Null).unwrap_err().error(),
        Errno::EEXIST
    );
    assert_eq!(class.devices().len(), 5);
    assert!(
        dev_root
            .lookup("null")
            .unwrap()
            .open(AccessMode::O_RDWR, StatusFlags::empty())
            .unwrap()
            .is_ok()
    );

    for dev in class.devices() {
        let id = device::Device::id(dev.as_ref());
        aster_device::remove(&dev).unwrap();
        char::unregister(id).unwrap();
    }
    assert!(sysnode("devices/virtual/mem").is_none());

    // The backend is registered first; a later sysfs conflict must undo it.
    let blocker = ClassDevice::builder(class, "null", MemFile::Null).build();
    aster_device::add(&blocker).unwrap();
    let null_id = DeviceId::new(MajorId::new(1), MinorId::new(3));
    assert_eq!(
        super::add_device(MemFile::Null).unwrap_err().error(),
        Errno::EEXIST
    );
    assert!(device::lookup(DeviceType::Char, null_id).is_none());
    assert!(dev_root.lookup("null").is_err());
    assert_eq!(class.devices().len(), 1);
    aster_device::remove(&blocker).unwrap();

    // A pre-existing file causes the real devtmpfs hook to fail. It survives
    // rollback, and neither the number registry nor sysfs retain this attempt.
    let conflict = dev_root
        .create("null", InodeType::File, mkmod!(u+rw))
        .unwrap();
    assert_eq!(
        super::add_device(MemFile::Null).unwrap_err().error(),
        Errno::EIO
    );
    assert!(device::lookup(DeviceType::Char, null_id).is_none());
    assert!(sysnode("class/mem/null").is_none());
    assert!(sysnode("dev/char/1:3").is_none());
    assert!(sysnode("devices/virtual/mem").is_none());
    assert_eq!(dev_root.lookup("null").unwrap().ino(), conflict.ino());
    dev_root.unlink("null", &conflict).unwrap();

    super::add_device(MemFile::Null).unwrap();
    let dev = class.devices().pop().unwrap();
    aster_device::remove(&dev).unwrap();
    char::unregister(null_id).unwrap();
    assert!(class.devices().is_empty());
}
