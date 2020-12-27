use log::debug;
use num_traits::FromPrimitive;
use readstat_sys;
use std::ffi::CStr;
use std::os::raw::{c_char, c_int, c_void};

use crate::rs::{ReadStatData, ReadStatReader, ReadStatVar, ReadStatVarMetadata, ReadStatVarType};

const ROWS: usize = 1000;

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

    // get row count and variable count
    let rc: c_int = unsafe { readstat_sys::readstat_get_row_count(metadata) };
    let vc: c_int = unsafe { readstat_sys::readstat_get_var_count(metadata) };

    // insert into ReadStatData struct
    d.row_count = rc;
    d.var_count = vc;

    debug!("d struct is {:#?}", d);
    debug!("row_count is {:#?}", d.row_count);
    debug!("var_count is {:#?}", d.var_count);

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

    // get type and name
    let var_type = match FromPrimitive::from_i32(unsafe {
        readstat_sys::readstat_variable_get_type(variable) as i32
    }) {
        Some(t) => t,
        None => ReadStatVarType::Unknown,
    };
    let var_name = unsafe {
        CStr::from_ptr(readstat_sys::readstat_variable_get_name(variable))
            .to_str()
            .unwrap()
            .to_owned()
    };

    debug!("d struct is {:#?}", d);
    debug!("var type pushed is {:#?}", var_type);
    debug!("var pushed is {:#?}", &var_name);

    // insert into BTreeMap within ReadStatData struct
    d.vars
        .insert(ReadStatVarMetadata::new(index, var_name), var_type);

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

    // if first row and first variable, allocate row and rows
    if obs_index == 0 && var_index == 0 {
        // Vec containing a single row, needs capacity = number of variables
        d.row = Vec::with_capacity(d.var_count as usize);
        // Vec containing all rows, needs capacity = number of rows
        // d.rows = Vec::with_capacity(d.row_count as usize);
        // Allocate rows
        d.rows = match d.reader {
            ReadStatReader::Streaming => {
                if d.row_count < ROWS as i32 {
                    Vec::with_capacity(d.row_count as usize)
                } else {
                    Vec::with_capacity(ROWS)
                }
            }
            ReadStatReader::InMemory => Vec::with_capacity(d.row_count as usize),
        }
    }

    debug!("var_index is {:#?}", var_index);
    debug!("value_type is {:#?}", value_type);
    debug!("is_missing {:#?}", is_missing);

    // get value and push into row within ReadStatData struct
    if is_missing == 0 {
        let value: ReadStatVar = match value_type {
            readstat_sys::readstat_type_e_READSTAT_TYPE_STRING
            | readstat_sys::readstat_type_e_READSTAT_TYPE_STRING_REF => {
                ReadStatVar::ReadStat_String(unsafe {
                    CStr::from_ptr(readstat_sys::readstat_string_value(value))
                        .to_str()
                        .unwrap()
                        .to_owned()
                })
            }
            readstat_sys::readstat_type_e_READSTAT_TYPE_INT8 => {
                ReadStatVar::ReadStat_i8(unsafe { readstat_sys::readstat_int8_value(value) })
            }
            readstat_sys::readstat_type_e_READSTAT_TYPE_INT16 => {
                ReadStatVar::ReadStat_i16(unsafe { readstat_sys::readstat_int16_value(value) })
            }
            readstat_sys::readstat_type_e_READSTAT_TYPE_INT32 => {
                ReadStatVar::ReadStat_i32(unsafe { readstat_sys::readstat_int32_value(value) })
            }
            readstat_sys::readstat_type_e_READSTAT_TYPE_FLOAT => {
                ReadStatVar::ReadStat_f32(unsafe { readstat_sys::readstat_float_value(value) })
            }
            readstat_sys::readstat_type_e_READSTAT_TYPE_DOUBLE => {
                ReadStatVar::ReadStat_f64(unsafe { readstat_sys::readstat_double_value(value) })
            }
            // exhaustive
            _ => unreachable!(),
        };

        debug!("value is {:#?}", value);

        // push into row
        d.row.push(value);
    } else {
        // For now represent missing values as the unit type
        // When serializing to csv (which is the only output type at the moment),
        //   the unit type is serialized as a missing value
        // For example, the following SAS dataset
        //   | id | name | age |
        //   | 4 | Alice | .   |
        //   | 5 | ""    | 30  |
        // would be serialized as the following in csv
        //   id,name,age
        //   4,Alice,,
        //   5,,30
        // And thus any missingness treatment is in fact handled by the tool that
        // consumes the csv file
        let value = ReadStatVar::ReadStat_Missing(());
        debug!("value is {:#?}", value);

        // push into row
        d.row.push(value);
    }

    // if last variable for a row, push into rows within ReadStatData struct
    if var_index == d.var_count - 1 {
        // collecting ALL rows into memory before ever writing
        // TODO: benchmark changes if were to push (for example) 1,000 rows at a time
        //       into the Vector and then flush to disk in a quasi-streaming fashion
        d.rows.push(d.row.clone());
        // clear row after pushing into rows; has no effect on capacity
        d.row.clear();
    }

    match d.reader {
        ReadStatReader::Streaming => {
            // if rows = buffer limit and last variable then go ahead and write
            if (obs_index % (ROWS as i32 - 1) == 0 || obs_index == d.row_count - 1)
                && var_index == d.var_count - 1
            {
                match d.write() {
                    Ok(()) => (),
                    // Err(e) => d.errors.push(format!("{:#?}", e)),
                    // For now just swallow any errors when writing
                    Err(_) => (),
                };
                d.rows.clear();
            }
        }
        ReadStatReader::InMemory => (),
    }

    ReadStatHandler::READSTAT_HANDLER_OK as c_int
}
