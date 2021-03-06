#![deny(unsafe_code)]


/// This is a low-level crate meant for use by the dbus crate.
///
/// No stability guarantees for this crate.

pub use dbus_native_channel::{machineid, address, authentication};

pub mod message;

pub mod types;

pub mod marshalled;

pub mod strings {
    //! Re-export of the dbus_strings crate
    pub use dbus_strings::*;
}
