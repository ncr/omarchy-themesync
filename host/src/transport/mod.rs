//! How a packet gets to a watch: a real BLE link, the resident daemon's socket, or the
//! in-process simulator. `sync` picks the first one that works, in that order.

pub mod ble;
pub mod ipc;
pub mod sim;
