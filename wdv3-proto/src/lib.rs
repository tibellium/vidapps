pub mod license_protocol {
    include!(concat!(env!("OUT_DIR"), "/license_protocol.rs"));
}

pub use license_protocol::*;
