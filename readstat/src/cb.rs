use arrow::array::{
    ArrayBuilder,
    ArrayRef,
    Int8Builder,
    Int16Builder,
    Int32Builder,
    Float32Builder,
    Float64Builder,
    StringBuilder
};
use arrow::datatypes::{DataType, Field, Schema};
use arrow::record_batch::RecordBatch;
use chrono::{Duration, NaiveDateTime, TimeZone, Utc};
use lexical::{to_string, parse};
use log::debug;
use num_traits::FromPrimitive;
use readstat_sys;
use std::ffi::CStr;
use std::os::raw::{c_char, c_int, c_void};
use std::sync::Arc;

use crate::formats;
use crate::rs::{
    ReadStatCompress, ReadStatData, ReadStatEndian, ReadStatFormatClass, ReadStatVar,
    ReadStatVarIndexAndName, ReadStatVarMetadata, ReadStatVarType, ReadStatVarTypeClass,
};
use crate::Reader;

const DIGITS: usize = 14;
const ROWS: usize = 100000;
const SEC_SHIFT: i64 = 315619200;
const SEC_PER_HOUR: i64 = 86400;

// C types
#[allow(dead_code)]
#[derive(Copy, Clone, Debug)]
#[repr(C)]
enum ReadStatHandler {
    READSTAT_HANDLER_OK,
    READSTAT_HANDLER_ABORT,
    READSTAT_HANDLER_SKIP_VARIABLE,
}

// C callback functions

// TODO: May need a version of handle_metadata that only gets metadata
//       and a version that does very little and instead metadata handling occurs
//       in handle_value function
//       As an example see the below from the readstat binary
//         https://github.com/WizardMac/ReadStat/blob/master/src/bin/readstat.c#L98
pub extern "C" fn handle_metadata(
    metadata: *mut readstat_sys::readstat_metadata_t,
    ctx: *mut c_void,
) -> c_int {
    // dereference ctx pointer
    let mut d = unsafe { &mut *(ctx as *mut ReadStatData) };

    // get metadata
    let rc: c_int = unsafe { readstat_sys::readstat_get_row_count(metadata) };
    let vc: c_int = unsafe { readstat_sys::readstat_get_var_count(metadata) };
    let table_name_ptr = unsafe { readstat_sys::readstat_get_table_name(metadata) };
    let table_name = if table_name_ptr == std::ptr::null() {
        String::new()
    } else {
        unsafe { CStr::from_ptr(table_name_ptr).to_str().unwrap().to_owned() }
    };
    let file_label_ptr = unsafe { readstat_sys::readstat_get_file_label(metadata) };
    let file_label = if file_label_ptr == std::ptr::null() {
        String::new()
    } else {
        unsafe { CStr::from_ptr(file_label_ptr).to_str().unwrap().to_owned() }
    };
    let file_encoding_ptr = unsafe { readstat_sys::readstat_get_file_encoding(metadata) };
    let file_encoding = if file_encoding_ptr == std::ptr::null() {
        String::new()
    } else {
        unsafe {
            CStr::from_ptr(file_encoding_ptr)
                .to_str()
                .unwrap()
                .to_owned()
        }
    };
    let version: c_int = unsafe { readstat_sys::readstat_get_file_format_version(metadata) };
    let is64bit = unsafe { readstat_sys::readstat_get_file_format_is_64bit(metadata) };
    let ct = NaiveDateTime::from_timestamp(
        unsafe { readstat_sys::readstat_get_creation_time(metadata) },
        0,
    )
    .format("%Y-%m-%d %H:%M:%S")
    .to_string();
    let mt = NaiveDateTime::from_timestamp(
        unsafe { readstat_sys::readstat_get_modified_time(metadata) },
        0,
    )
    .format("%Y-%m-%d %H:%M:%S")
    .to_string();
    let compression = match FromPrimitive::from_i32(unsafe {
        readstat_sys::readstat_get_compression(metadata) as i32
    }) {
        Some(t) => t,
        None => ReadStatCompress::None,
    };
    let endianness = match FromPrimitive::from_i32(unsafe {
        readstat_sys::readstat_get_endianness(metadata) as i32
    }) {
        Some(t) => t,
        None => ReadStatEndian::None,
    };
    d.cols = Vec::with_capacity(vc as usize);

    debug!("row_count is {}", rc);
    debug!("var_count is {}", vc);
    debug!("table_name is {}", &table_name);
    debug!("file_label is {}", &file_label);
    debug!("file_encoding is {}", &file_encoding);
    debug!("version is {}", version);
    debug!("is64bit is {}", is64bit);
    debug!("creation_time is {}", &ct);
    debug!("modified_time is {}", &mt);
    debug!("compression is {:#?}", &compression);
    debug!("endianness is {:#?}", &endianness);

    // insert into ReadStatData struct
    d.row_count = rc;
    d.var_count = vc;
    d.table_name = table_name;
    d.file_label = file_label;
    d.file_encoding = file_encoding;
    d.version = version;
    d.is64bit = is64bit;
    d.creation_time = ct;
    d.modified_time = mt;
    d.compression = compression;
    d.endianness = endianness;

    // debug!("d struct is {:#?}", d);

    ReadStatHandler::READSTAT_HANDLER_OK as c_int
}

pub extern "C" fn handle_variable(
    index: c_int,
    variable: *mut readstat_sys::readstat_variable_t,
    #[allow(unused_variables)] val_labels: *const c_char,
    ctx: *mut c_void,
) -> c_int {
    // dereference ctx pointer
    let d = unsafe { &mut *(ctx as *mut ReadStatData) };

    // get variable metadata
    let var_type = match FromPrimitive::from_i32(unsafe {
        readstat_sys::readstat_variable_get_type(variable) as i32
    }) {
        Some(t) => t,
        None => ReadStatVarType::Unknown,
    };
    let var_type_class = match FromPrimitive::from_i32(unsafe {
        readstat_sys::readstat_variable_get_type_class(variable) as i32
    }) {
        Some(t) => t,
        None => ReadStatVarTypeClass::Numeric,
    };
    let var_name_ptr = unsafe { readstat_sys::readstat_variable_get_name(variable) };
    let var_name = if var_name_ptr == std::ptr::null() {
        String::new()
    } else {
        unsafe { CStr::from_ptr(var_name_ptr).to_str().unwrap().to_owned() }
    };
    let var_label_ptr = unsafe { readstat_sys::readstat_variable_get_label(variable) };
    let var_label = if var_label_ptr == std::ptr::null() {
        String::new()
    } else {
        unsafe { CStr::from_ptr(var_label_ptr).to_str().unwrap().to_owned() }
    };

    let var_format_ptr = unsafe { readstat_sys::readstat_variable_get_format(variable) };
    let var_format = if var_format_ptr == std::ptr::null() {
        String::new()
    } else {
        unsafe { CStr::from_ptr(var_format_ptr).to_str().unwrap().to_owned() }
    };

    let var_format_class = formats::match_var_format(&var_format);

    debug!("var_type is {:#?}", &var_type);
    debug!("var_type_class is {:#?}", &var_type_class);
    debug!("var_name is {}", &var_name);
    debug!("var_label is {}", &var_label);
    debug!("var_format is {}", &var_format);
    debug!("var_format_class is {:#?}", &var_format_class);

    // insert into BTreeMap within ReadStatData struct
    d.vars.insert(
        ReadStatVarIndexAndName::new(index, var_name.clone()),
        ReadStatVarMetadata::new(
            var_type,
            var_type_class,
            var_label,
            var_format,
            var_format_class,
        ),
    );


    // Build up Schema
    // TODO - need to handle Dates, Times, and Datetimes
    let var_dt = match &var_type {
        ReadStatVarType::String | ReadStatVarType::StringRef | ReadStatVarType::Unknown => {
            DataType::Utf8
        }
        ReadStatVarType::Int8 | ReadStatVarType::Int16 => DataType::Int16,
        ReadStatVarType::Int32 => DataType::Int32,
        ReadStatVarType::Float => DataType::Float32,
        ReadStatVarType::Double => DataType::Float64,
    };

    d.schema =
        Schema::try_merge(vec![d.schema.clone(), Schema::new(vec![Field::new(
            &var_name, var_dt, true,
        )])])
        .unwrap();

    /*
    match d.reader {
        Reader::stream => d.cols.push(Vec::with_capacity(std::cmp::min(ROWS, d.row_count as usize))),
        Reader::mem => d.cols.push(Vec::with_capacity(d.row_count as usize)),
    };
    */

    let array: Box<dyn ArrayBuilder> = match &var_type {
        ReadStatVarType::String
        | ReadStatVarType::StringRef
        | ReadStatVarType::Unknown => Box::new(StringBuilder::new(std::cmp::min(ROWS, d.row_count as usize))),
        ReadStatVarType::Int8 => Box::new(Int8Builder::new(std::cmp::min(ROWS, d.row_count as usize))),
        ReadStatVarType::Int16 => Box::new(Int16Builder::new(std::cmp::min(ROWS, d.row_count as usize))),
        ReadStatVarType::Int32 => Box::new(Int32Builder::new(std::cmp::min(ROWS, d.row_count as usize))),
        ReadStatVarType::Float => Box::new(Float32Builder::new(std::cmp::min(ROWS, d.row_count as usize))),
        ReadStatVarType::Double => Box::new(Float64Builder::new(std::cmp::min(ROWS, d.row_count as usize))),
    };

    // TODO - implement Debug for array
    // debug!("array is {:#?}", array);
    
    d.cols.push(array);

    // debug!("d struct is {:#?}", d);

    ReadStatHandler::READSTAT_HANDLER_OK as c_int
}

pub extern "C" fn handle_value(
    #[allow(unused_variables)] obs_index: c_int,
    variable: *mut readstat_sys::readstat_variable_t,
    value: readstat_sys::readstat_value_t,
    ctx: *mut c_void,
) -> c_int {
    // dereference ctx pointer
    let d = unsafe { &mut *(ctx as *mut ReadStatData) };

    // get index, type, and missingness
    let var_index: c_int = unsafe { readstat_sys::readstat_variable_get_index(variable) };
    let value_type: readstat_sys::readstat_type_t =
        unsafe { readstat_sys::readstat_value_type(value) };
    let is_missing: c_int = unsafe { readstat_sys::readstat_value_is_system_missing(value) };

    debug!("row_count is {}", d.row_count);
    debug!("var_count is {}", d.var_count);
    debug!("obs_index is {}", obs_index);
    debug!("var_index is {}", var_index);
    debug!("value_type is {:#?}", &value_type);
    debug!("is_missing is {}", is_missing);

    // get value and push into cols
    match value_type {
        readstat_sys::readstat_type_e_READSTAT_TYPE_STRING
        | readstat_sys::readstat_type_e_READSTAT_TYPE_STRING_REF => {
            // get value
            let value = unsafe {
                CStr::from_ptr(readstat_sys::readstat_string_value(value))
                    .to_str()
                    .unwrap()
                    .to_owned()
            };
            // debug
            debug!("value is {:#?}", &value);
            // append to builder
            if is_missing == 0 {
                d.cols[var_index as usize].as_any_mut().downcast_mut::<StringBuilder>().unwrap().append_value(value.clone()).unwrap();
            } else {
                d.cols[var_index as usize].as_any_mut().downcast_mut::<StringBuilder>().unwrap().append_null().unwrap();
            }
        },
        readstat_sys::readstat_type_e_READSTAT_TYPE_INT8 => {
            // get value
            let value = unsafe { readstat_sys::readstat_int8_value(value) };
            // debug
            debug!("value is {:#?}", value);
            // append to builder
            if is_missing == 0 {
                d.cols[var_index as usize].as_any_mut().downcast_mut::<Int8Builder>().unwrap().append_value(value).unwrap();
            } else {
                d.cols[var_index as usize].as_any_mut().downcast_mut::<Int8Builder>().unwrap().append_null().unwrap();
            }
        },
        readstat_sys::readstat_type_e_READSTAT_TYPE_INT16 => {
            // get value
            let value = unsafe { readstat_sys::readstat_int16_value(value) };
            // debug
            debug!("value is {:#?}", value);
            // append to builder
            if is_missing == 0 {
                d.cols[var_index as usize].as_any_mut().downcast_mut::<Int16Builder>().unwrap().append_value(value).unwrap();
            } else {
                d.cols[var_index as usize].as_any_mut().downcast_mut::<Int16Builder>().unwrap().append_null().unwrap();
            }
        },
        readstat_sys::readstat_type_e_READSTAT_TYPE_INT32 => {
            // get value
            let value = unsafe { readstat_sys::readstat_int32_value(value) };
            // debug
            debug!("value is {:#?}", value);
            // append to builder
            if is_missing == 0 {
                d.cols[var_index as usize].as_any_mut().downcast_mut::<Int32Builder>().unwrap().append_value(value).unwrap();
            } else {
                d.cols[var_index as usize].as_any_mut().downcast_mut::<Int32Builder>().unwrap().append_null().unwrap();
            }
        },
        readstat_sys::readstat_type_e_READSTAT_TYPE_FLOAT => {
            // Format as string to truncate float to only contain 14 decimal digits
            // Parse back into float so that the trailing zeroes are trimmed when serializing
            // TODO: Is there an alternative that does not require conversion from and to a float?  // get value
            let value = unsafe { readstat_sys::readstat_float_value(value) };
            let value = lexical::parse::<f32, _>(format!("{1:.0$}", DIGITS, lexical::to_string(value))).unwrap();
            // debug
            debug!("value is {:#?}", value);
            // append to builder
            if is_missing == 0 {
                d.cols[var_index as usize].as_any_mut().downcast_mut::<Float32Builder>().unwrap().append_value(value).unwrap();
            } else {
                d.cols[var_index as usize].as_any_mut().downcast_mut::<Float32Builder>().unwrap().append_null().unwrap();
            }
        },
        readstat_sys::readstat_type_e_READSTAT_TYPE_DOUBLE => {
            let value = unsafe { readstat_sys::readstat_double_value(value) };
            let value = lexical::parse::<f64, _>(format!("{1:.0$}", DIGITS, lexical::to_string(value))).unwrap();
            // debug
            debug!("value is {:#?}", value);
            // append to builder
            if is_missing == 0 {
                d.cols[var_index as usize].as_any_mut().downcast_mut::<Float64Builder>().unwrap().append_value(value).unwrap();
            } else {
                d.cols[var_index as usize].as_any_mut().downcast_mut::<Float64Builder>().unwrap().append_null().unwrap();
            }
        },
        // exhaustive
        _ => unreachable!(),
    };


    // TODO: check if date/datetime format
    // Rather than have a massive set of string comparisons, may want to convert the original strings to enums and then match on the enums
    // Probably can move the date/datetime checks out of the handle_value function and into the handle_variable function
    // The value conversion, obviously, would still need to occur here within handle_value
    //let v = d.get_readstatvarmeta_from_index(var_index);

    /*
    let value = match v.var_format_class {
        Some(ReadStatFormatClass::Date) => {
            let f = match value {
                ReadStatVar::ReadStat_f64(f) => f as i64,
                _ => 0 as i64,
            };
            ReadStatVar::ReadStat_Date(
                Utc.timestamp(f * SEC_PER_HOUR, 0)
                    .checked_sub_signed(Duration::seconds(SEC_SHIFT))
                    .unwrap()
                    .naive_utc()
                    .date(),
            )
        }
        Some(ReadStatFormatClass::DateTime) => {
            let f = match value {
                ReadStatVar::ReadStat_f64(f) => f as i64,
                _ => 0 as i64,
            };
            ReadStatVar::ReadStat_DateTime(
                Utc.timestamp(f, 0)
                    .checked_sub_signed(Duration::seconds(SEC_SHIFT))
                    .unwrap(),
            )
        }
        Some(ReadStatFormatClass::Time) => {
            let f = match value {
                ReadStatVar::ReadStat_f64(f) => f as i64,
                _ => 0 as i64,
            };
            ReadStatVar::ReadStat_Time(
                Utc.timestamp(f, 0)
                    .checked_sub_signed(Duration::seconds(SEC_SHIFT))
                    .unwrap()
                    .naive_utc()
                    .time(),
            )
        }
        None => value,
    };
    */

    // if last variable for a row, check to see if data should be finalized and written
    if var_index == d.var_count - 1 {

        match d.reader {
            // if rows = buffer limit and last variable then go ahead and write
            Reader::stream
                if (((obs_index + 1) % ROWS as i32 == 0) && (obs_index != 0))
                    || obs_index == (d.row_count - 1) =>
            {
                let arrays: Vec<ArrayRef> = d
                    .cols
                    .iter_mut()
                    .map(|builder| builder.finish())
                    .collect();

                d.batch = RecordBatch::try_new(
                    Arc::new(d.schema.clone()),
                    arrays
                ).unwrap();

                match d.write() {
                    Ok(()) => (),
                    // Err(e) => d.errors.push(format!("{:#?}", e)),
                    // TODO: what to do with writing errors?
                    //       could include an errors container on the ReadStatData struct
                    //         and carry the errors generated to be accessed by the end user
                    //       or could simply dump the errors to standard out or even write them
                    //         to a separate file
                    // For now just swallow any errors when writing
                    Err(_) => (),
                };
            },
            Reader::mem if obs_index == (d.row_count - 1) => {
                match d.write() {
                    Ok(()) => (),
                    // Err(e) => d.errors.push(format!("{:#?}", e)),
                    // TODO: what to do with writing errors?
                    //       could include an errors container on the ReadStatData struct
                    //         and carry the errors generated to be accessed by the end user
                    //       or could simply dump the errors to standard out or even write them
                    //         to a separate file
                    // For now just swallow any errors when writing
                    Err(_) => (),
                };
            }
            _ => (),
        }
    }

    ReadStatHandler::READSTAT_HANDLER_OK as c_int
}
