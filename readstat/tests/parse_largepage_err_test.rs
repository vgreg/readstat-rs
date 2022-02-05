use path_abs::PathAbs;
use std::env;

#[test]
fn parse_largepage_err() {
    // setup path
    let project_dir = PathAbs::new(env!("CARGO_MANIFEST_DIR")).unwrap();
    let data_dir = project_dir.as_path().join("tests").join("data");
    let sas_path = data_dir.join("rand_ds_largepage_err.sas7bdat");
    let rsp = readstat::ReadStatPath::new(sas_path, None, None, false).unwrap();

    // parse sas7bdat
    let mut d = readstat::ReadStatData::new(rsp)
        .set_reader(Some(readstat::Reader::mem))
        .set_no_progress(true)
        .set_no_write(true);
    let error = d.get_metadata(false).unwrap();

    assert_eq!(error, readstat_sys::readstat_error_e_READSTAT_OK as u32);

    // row count
    assert_eq!(d.metadata.row_count, 2000);

    // variable count
    assert_eq!(d.metadata.var_count, 110);

    // table name
    assert_eq!(d.metadata.table_name, String::from("RAND_DS_LARGEPAGE_ERR"));

    // table label
    assert_eq!(d.metadata.file_label, String::from(""));

    // file encoding
    assert_eq!(d.metadata.file_encoding, String::from("UTF-8"));

    // format version
    assert_eq!(d.metadata.version, 9);

    // bitness
    assert_eq!(d.metadata.is64bit, 1);

    // creation time
    assert_eq!(d.metadata.creation_time, "2021-07-25 22:02:02");

    // modified time
    assert_eq!(d.metadata.modified_time, "2021-07-25 22:02:02");

    // compression
    assert!(matches!(
        d.metadata.compression,
        readstat::ReadStatCompress::None
    ));

    // endianness
    assert!(matches!(
        d.metadata.endianness,
        readstat::ReadStatEndian::Little
    ));
}
