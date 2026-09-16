// SPDX-License-Identifier: MPL-2.0

use alloc::{sync::Arc, vec};

use ostd::{
    mm::{VmReader, VmWriter},
    prelude::ktest,
};
use spin::Once;

use super::{Attr, AttrTable, TyErasedAttr};
use crate::{AnyDevice, Class, ClassDevice, ClassHandle, Error};

struct TableClass;

impl Class for TableClass {
    const NAME: &'static str = "attr_table_test";
    type Device = Arc<AttrTable>;
}

fn device(table: &Arc<AttrTable>) -> Arc<ClassDevice<TableClass>> {
    static CLASS: Once<Arc<ClassHandle<TableClass>>> = Once::new();
    crate::init_for_ktest();
    let class = CLASS.call_once(|| crate::register_class(TableClass).unwrap());
    ClassDevice::builder(class, "table", table.clone()).build()
}

const NAME: Attr<dyn AnyDevice> = Attr::ro("name", |dev, writer| {
    writeln!(writer, "{}", dev.base().name())?;
    Ok(())
});
const EXTRA: Attr<dyn AnyDevice> = Attr::ro("extra", |_, writer| {
    writeln!(writer, "extra")?;
    Ok(())
});

#[ktest]
fn attribute_snapshots_keep_surviving_ids() {
    let table = Arc::new(AttrTable::new());
    let dev = device(&table);
    table.add(vec![TyErasedAttr::from_dyn(&NAME)]).unwrap();
    let original = table.set();
    let name_id = original.get("name").unwrap().id();

    table.add(vec![TyErasedAttr::from_dyn(&EXTRA)]).unwrap();
    let expanded = table.set();
    assert_eq!(expanded.len(), 2);
    assert_eq!(expanded.get("name").unwrap().id(), name_id);
    assert!(!original.contains("extra"));

    table.remove(&["extra", "missing"]);
    assert!(expanded.contains("extra"));
    assert!(!table.set().contains("extra"));
    assert_eq!(table.set().get("name").unwrap().id(), name_id);

    let mut bytes = [0; 16];
    let mut writer = VmWriter::from(&mut bytes[..]).to_fallible();
    assert!(matches!(
        table.show(dev.as_ref(), "extra", 0, &mut writer),
        Err(aster_systree::Error::NotFound)
    ));
    let written = table.show(dev.as_ref(), "name", 0, &mut writer).unwrap();
    assert_eq!(&bytes[..written], b"table\n");
}

#[ktest]
fn failed_attribute_batches_leave_no_partial_entries() {
    let table = AttrTable::new();
    table.add(vec![TyErasedAttr::from_dyn(&NAME)]).unwrap();
    assert!(matches!(
        table.add(vec![
            TyErasedAttr::from_dyn(&EXTRA),
            TyErasedAttr::from_dyn(&NAME)
        ]),
        Err(Error::NameConflict)
    ));
    assert!(matches!(
        table.add(vec![
            TyErasedAttr::from_dyn(&EXTRA),
            TyErasedAttr::from_dyn(&EXTRA)
        ]),
        Err(Error::NameConflict)
    ));
    let invalid = Attr::ro("bad/name", |_: &dyn AnyDevice, _| Ok(()));
    assert!(matches!(
        table.add(vec![
            TyErasedAttr::from_dyn(&EXTRA),
            TyErasedAttr::from_dyn(&invalid)
        ]),
        Err(Error::InvalidName)
    ));
    assert_eq!(table.set().len(), 1);
    assert!(!table.set().contains("extra"));
}

#[ktest]
fn attribute_capacity_failure_preserves_table_and_callbacks() {
    // Distinct static names let this test fill the ID space without leaking
    // allocations to satisfy the attribute declarations' static lifetime.
    static NAMES: [[u8; 2]; 256] = {
        let mut names = [[0; 2]; 256];
        let mut index = 0;
        while index < names.len() {
            names[index] = [b'a' + (index / 16) as u8, b'a' + (index % 16) as u8];
            index += 1;
        }
        names
    };
    let attrs = NAMES
        .iter()
        .map(|name| {
            let name = core::str::from_utf8(name).unwrap();
            TyErasedAttr::from_dyn(&Attr::ro(name, |_: &dyn AnyDevice, _| Ok(())))
        })
        .collect();
    let table = AttrTable::new();
    table.add(attrs).unwrap();
    assert!(matches!(
        table.add(vec![TyErasedAttr::from_dyn(&NAME)]),
        Err(Error::ResourceUnavailable)
    ));
    assert_eq!(table.set().len(), NAMES.len());
    assert!(!table.set().contains("name"));

    let retained_id = table.set().get("pp").unwrap().id();
    table.remove(&["aa"]);
    table.add(vec![TyErasedAttr::from_dyn(&NAME)]).unwrap();
    assert_eq!(table.set().len(), NAMES.len());
    assert_eq!(table.set().get("pp").unwrap().id(), retained_id);
}

#[ktest]
fn attribute_callbacks_can_remove_themselves() {
    const ATTRS: &[Attr<ClassDevice<TableClass>>] = &[
        Attr::ro("read_once", |dev, writer| {
            dev.payload().remove(&["read_once"]);
            writeln!(writer, "done")?;
            Ok(())
        }),
        Attr::wo("write_once", |dev, value| {
            if value != "done" {
                return Err(Error::InvalidValue);
            }
            dev.payload().remove(&["write_once"]);
            Ok(())
        }),
    ];
    let table = Arc::new(AttrTable::new());
    let dev = device(&table);
    table.add(TyErasedAttr::from_typed_slice(ATTRS)).unwrap();
    let mut bytes = [0; 8];
    let mut writer = VmWriter::from(&mut bytes[..]).to_fallible();
    let written = table
        .show(dev.as_ref(), "read_once", 0, &mut writer)
        .unwrap();
    assert_eq!(&bytes[..written], b"done\n");
    let mut reader = VmReader::from(&b"done\n"[..]).to_fallible();
    assert_eq!(
        table
            .store(dev.as_ref(), "write_once", &mut reader)
            .unwrap(),
        5
    );
    assert!(table.set().is_empty());
}
