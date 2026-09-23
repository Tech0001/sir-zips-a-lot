use std::{fs, io::Read};

use sir_zips_a_lot::archive_order;
use tempfile::tempdir;
use zip::ZipArchive;

#[test]
fn extracts_order_contents_without_a_second_parent_folder() {
    let temp = tempdir().unwrap();
    let order = temp.path().join("Order 100 — café");
    fs::create_dir_all(order.join("empty")).unwrap();
    fs::create_dir_all(order.join("artwork")).unwrap();
    fs::write(order.join("artwork/proof.txt"), "Approved ✓").unwrap();
    fs::write(order.join("order.txt"), "Order details").unwrap();

    let archive = archive_order(&order, &temp.path().join("out")).unwrap();
    assert_eq!(archive.file_name().unwrap(), "Order 100 — café.zip");
    let mut zip = ZipArchive::new(fs::File::open(archive).unwrap()).unwrap();
    let mut names: Vec<_> = zip.file_names().collect();
    names.sort_unstable();
    assert_eq!(
        names,
        ["artwork/", "artwork/proof.txt", "empty/", "order.txt"]
    );
    assert!(zip.by_name("empty/").unwrap().is_dir());
    let mut contents = String::new();
    zip.by_name("artwork/proof.txt")
        .unwrap()
        .read_to_string(&mut contents)
        .unwrap();
    assert_eq!(contents, "Approved ✓");
    assert_eq!(
        fs::read_to_string(order.join("artwork/proof.txt")).unwrap(),
        contents
    );

    // Windows Extract All uses the ZIP name as its destination folder.
    let extracted = temp.path().join("extracted/Order 100 — café");
    zip.extract(&extracted).unwrap();
    assert_eq!(
        fs::read_to_string(extracted.join("artwork/proof.txt")).unwrap(),
        contents
    );
    assert_eq!(
        fs::read_to_string(extracted.join("order.txt")).unwrap(),
        "Order details"
    );
    assert!(extracted.join("empty").is_dir());
    assert!(!extracted.join("Order 100 — café").exists());
}

#[test]
fn empty_order_produces_a_valid_zip_without_a_wrapper_directory() {
    let temp = tempdir().unwrap();
    let order = temp.path().join("empty-order");
    fs::create_dir(&order).unwrap();
    let archive = archive_order(&order, &temp.path().join("out")).unwrap();
    let zip = ZipArchive::new(fs::File::open(archive).unwrap()).unwrap();
    assert!(zip.is_empty());
    assert!(order.is_dir());
}

#[test]
fn archives_single_files_and_refuses_to_overwrite() {
    let temp = tempdir().unwrap();
    let order = temp.path().join("order.csv");
    let output = temp.path().join("out");
    fs::write(&order, "item,quantity\nshirt,12\n").unwrap();
    let archive = archive_order(&order, &output).unwrap();
    let before = fs::read(&archive).unwrap();
    assert_eq!(archive.file_name().unwrap(), "order.csv.zip");
    let mut zip = ZipArchive::new(fs::File::open(&archive).unwrap()).unwrap();
    assert_eq!(zip.file_names().collect::<Vec<_>>(), ["order.csv"]);
    let mut contents = String::new();
    zip.by_name("order.csv")
        .unwrap()
        .read_to_string(&mut contents)
        .unwrap();
    assert_eq!(contents, fs::read_to_string(&order).unwrap());
    assert!(archive_order(&order, &output).is_err());
    assert_eq!(fs::read(archive).unwrap(), before);
    assert_eq!(fs::read_dir(output).unwrap().count(), 1);
}

#[test]
fn rejects_destination_inside_the_order() {
    let temp = tempdir().unwrap();
    let order = temp.path().join("order");
    fs::create_dir(&order).unwrap();
    assert!(archive_order(&order, &order.join("out")).is_err());
    assert!(archive_order(&order, &order).is_err());
}

#[cfg(unix)]
#[test]
fn rejects_links_and_cleans_up_incomplete_archive() {
    let temp = tempdir().unwrap();
    let order = temp.path().join("order");
    let output = temp.path().join("out");
    fs::create_dir(&output).unwrap();
    fs::create_dir(&order).unwrap();
    fs::write(order.join("a.txt"), "included first").unwrap();
    std::os::unix::fs::symlink(temp.path(), order.join("z-link")).unwrap();
    assert!(archive_order(&order, &output).is_err());
    assert_eq!(fs::read_dir(output).unwrap().count(), 0);
}
