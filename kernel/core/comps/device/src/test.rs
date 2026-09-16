// SPDX-License-Identifier: MPL-2.0

//! Registration tests through the public device and systree interfaces.

use alloc::sync::Arc;
use core::sync::atomic::{AtomicUsize, Ordering};

use aster_systree::{SysBranchNode, SysNode, SysObj};
use ostd::{
    mm::{VmReader, VmWriter},
    prelude::ktest,
};
use spin::Once;

use crate::{AnyDevice, Attr, Class, ClassDevice, ClassHandle, Error};

struct TestClass;

impl Class for TestClass {
    const NAME: &'static str = "device_test";
    type Device = AtomicUsize;

    fn dev_attrs(&self) -> &'static [Attr<ClassDevice<Self>>] {
        CLASS_ATTRS
    }
}

const CLASS_ATTRS: &[Attr<ClassDevice<TestClass>>] = &[Attr::rw(
    "value",
    |dev, writer| {
        writeln!(writer, "{}", dev.load(Ordering::Relaxed))?;
        Ok(())
    },
    |dev, value| {
        dev.store(
            value.parse().map_err(|_| Error::InvalidValue)?,
            Ordering::Relaxed,
        );
        Ok(())
    },
)];

fn class() -> &'static Arc<ClassHandle<TestClass>> {
    static CLASS: Once<Arc<ClassHandle<TestClass>>> = Once::new();
    crate::init_for_ktest();
    CLASS.call_once(|| crate::register_class(TestClass).unwrap())
}

fn device(name: &'static str) -> Arc<ClassDevice<TestClass>> {
    ClassDevice::builder(class(), name, AtomicUsize::new(17)).build()
}

fn lookup(path: &str) -> Option<Arc<dyn SysObj>> {
    let mut node: Arc<dyn SysObj> = aster_systree::primary_tree().root().clone();
    for name in path.split('/').filter(|name| !name.is_empty()) {
        node = node.cast_to_branch()?.child(name)?;
    }
    Some(node)
}

const INSTANCE_ATTRS: &[Attr<ClassDevice<TestClass>>] = &[Attr::ro("instance", |_, writer| {
    writeln!(writer, "own")?;
    Ok(())
})];

const DUPLICATE_ATTRS: &[Attr<ClassDevice<TestClass>>] = &[Attr::ro("value", |_, _| Ok(()))];
const LINK_CONFLICT_ATTRS: &[Attr<ClassDevice<TestClass>>] =
    &[Attr::ro("subsystem", |_, _| Ok(()))];

#[ktest]
fn class_device_attributes_and_links() {
    let dev = ClassDevice::builder(class(), "attributes", AtomicUsize::new(17))
        .attrs(INSTANCE_ATTRS)
        .build();
    crate::add(&dev).unwrap();
    assert_eq!(dev.path(), "/devices/virtual/device_test/attributes");
    assert_eq!(dev.show_attr("value").unwrap(), "17\n");
    assert_eq!(dev.show_attr("instance").unwrap(), "own\n");
    let mut bytes = [0; 2];
    let mut writer = VmWriter::from(&mut bytes[..]).to_fallible();
    assert_eq!(dev.read_attr_at("value", 1, &mut writer).unwrap(), 2);
    assert_eq!(&bytes, b"7\n");
    let mut reader = VmReader::from(&b"23\n"[..]).to_fallible();
    assert_eq!(dev.write_attr("value", &mut reader).unwrap(), 3);
    assert_eq!(dev.load(Ordering::Relaxed), 23);
    assert_eq!(
        lookup("class/device_test/attributes")
            .unwrap()
            .cast_to_symlink()
            .unwrap()
            .target_path(),
        "../../devices/virtual/device_test/attributes"
    );
    assert_eq!(
        dev.child("subsystem")
            .unwrap()
            .cast_to_symlink()
            .unwrap()
            .target_path(),
        "../../../../class/device_test"
    );
    assert!(matches!(
        dev.remove_child("subsystem"),
        Err(aster_systree::Error::PermissionDenied)
    ));
    assert!(
        class()
            .devices()
            .iter()
            .any(|member| Arc::ptr_eq(member, &dev))
    );
    let erased: Arc<dyn AnyDevice> = dev.clone();
    crate::remove(&erased).unwrap();
    assert!(lookup("class/device_test/attributes").is_none());
    assert!(matches!(
        dev.show_attr("value"),
        Err(aster_systree::Error::IsDead)
    ));
}

#[ktest]
fn duplicate_registration_preserves_existing_device() {
    let first = device("duplicate");
    crate::add(&first).unwrap();
    assert!(matches!(crate::add(&first), Err(Error::AlreadyAdded)));
    let duplicate = device("duplicate");
    assert!(matches!(crate::add(&duplicate), Err(Error::NameConflict)));
    assert_eq!(
        lookup("devices/virtual/device_test/duplicate")
            .unwrap()
            .id(),
        first.id()
    );
    assert!(lookup("class/device_test/duplicate").is_some());
    crate::remove(&first).unwrap();
    let replacement = device("duplicate");
    crate::add(&replacement).unwrap();
    assert_ne!(first.id(), replacement.id());
    crate::remove(&replacement).unwrap();
}

#[ktest]
fn parent_lifetime_and_index_conflict() {
    let parent = device("parent");
    let child = ClassDevice::builder(class(), "child", AtomicUsize::new(0))
        .parent(parent.clone())
        .build();
    assert!(matches!(crate::add(&child), Err(Error::ParentNotAdded)));
    crate::add(&parent).unwrap();
    let child = ClassDevice::builder(class(), "child", AtomicUsize::new(0))
        .parent(parent.clone())
        .build();
    crate::add(&child).unwrap();
    assert_eq!(child.path(), "/devices/virtual/device_test/parent/child");
    assert_eq!(
        child
            .child("device")
            .unwrap()
            .cast_to_symlink()
            .unwrap()
            .target_path(),
        "../../parent"
    );
    assert!(matches!(crate::remove(&parent), Err(Error::HasChildren)));
    let conflicting = device("child");
    assert!(matches!(crate::add(&conflicting), Err(Error::NameConflict)));
    assert!(lookup("devices/virtual/device_test/child").is_none());
    assert!(lookup("class/device_test/child").is_some());
    crate::remove(&child).unwrap();
    assert!(parent.base().child_devices().is_empty());
    crate::remove(&parent).unwrap();
}

#[ktest]
fn invalid_attributes_leave_no_device() {
    let duplicate = ClassDevice::builder(class(), "bad_attrs", AtomicUsize::new(0))
        .attrs(DUPLICATE_ATTRS)
        .build();
    assert!(matches!(crate::add(&duplicate), Err(Error::NameConflict)));
    let link_conflict = ClassDevice::builder(class(), "link_conflict", AtomicUsize::new(0))
        .attrs(LINK_CONFLICT_ATTRS)
        .build();
    assert!(matches!(
        crate::add(&link_conflict),
        Err(Error::NameConflict)
    ));
    assert!(lookup("devices/virtual/device_test/bad_attrs").is_none());
    assert!(lookup("devices/virtual/device_test/link_conflict").is_none());
    assert!(lookup("class/device_test/link_conflict").is_none());
}

#[ktest]
#[should_panic(expected = "invalid SysTree node name")]
fn device_name_rejects_slash_at_construction() {
    device("bad/name");
}

#[ktest]
fn last_member_removes_virtual_glue() {
    struct TemporaryClass;
    impl Class for TemporaryClass {
        const NAME: &'static str = "temporary_device_test";
        type Device = ();
    }
    crate::init_for_ktest();
    let class = crate::register_class(TemporaryClass).unwrap();
    let first = ClassDevice::builder(&class, "first", ()).build();
    let second = ClassDevice::builder(&class, "second", ()).build();
    crate::add(&first).unwrap();
    crate::add(&second).unwrap();
    crate::remove(&first).unwrap();
    assert!(lookup("devices/virtual/temporary_device_test/second").is_some());
    crate::remove(&second).unwrap();
    assert!(lookup("devices/virtual/temporary_device_test").is_none());
    assert!(class.devices().is_empty());
}
